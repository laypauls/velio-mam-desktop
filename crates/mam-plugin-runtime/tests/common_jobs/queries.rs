//! 시험 단언용 DB 조회 — 실제 SQLite 를 읽을 뿐 쿼리를 흉내 내지 않는다.

use mam_db::Priority;
use mam_db::plugin_tasks;

use super::Env;

/// `plugin_tasks` 한 행
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub status: String,
    pub target: i64,
    pub done: i64,
    pub attempts: i64,
    pub last_error: Option<String>,
}

/// `jobs`(kind = plugin) 한 행
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub asset: i64,
    pub status: String,
    pub attempts: i64,
    pub not_before: i64,
}

impl Env {
    pub fn exec(&self, sql: &str) {
        let sql = sql.to_owned();
        self.pool.write(Priority::High, move |c| Ok(c.execute_batch(&sql)?)).unwrap();
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.pool.read(|c| Ok(c.query_row(sql, [], |r| r.get(0))?)).unwrap()
    }

    pub fn revision(&self, asset: i64) -> i64 {
        self.count(&format!("SELECT revision FROM media_assets WHERE asset_id = {asset}"))
    }

    pub fn max_event(&self) -> i64 {
        self.pool.read(|c| Ok(plugin_tasks::max_event_id(c)?)).unwrap()
    }

    pub fn cursor(&self, plugin: &str) -> Option<i64> {
        let p = plugin.to_owned();
        self.pool.read(move |c| Ok(plugin_tasks::feed_cursor(c, &p)?)).unwrap()
    }

    pub fn set_cursor(&self, plugin: &str, value: i64) {
        let p = plugin.to_owned();
        self.pool
            .write(Priority::High, move |c| {
                mam_db::plugin_settings::put(c, mam_db::plugin_settings::FEED_CURSOR, &p, &value.to_string()).unwrap();
                Ok(())
            })
            .unwrap();
    }

    pub fn task(&self, plugin: &str, provider: &str, asset: i64) -> Option<Task> {
        let (p, v) = (plugin.to_owned(), provider.to_owned());
        self.pool
            .read(move |c| {
                Ok(c.query_row(
                    "SELECT t.status, t.target_revision, t.done_revision, t.attempts, t.last_error FROM plugin_tasks t JOIN plugins p USING (plugin_ref) JOIN plugin_providers v USING (provider_ref) WHERE p.plugin_id=?1 AND v.name=?2 AND t.asset_id=?3",
                    (p, v, asset),
                    |r| Ok(Task { status: r.get(0)?, target: r.get(1)?, done: r.get(2)?, attempts: r.get(3)?, last_error: r.get(4)? }),
                )
                .ok())
            })
            .unwrap()
    }

    /// 작업 행 하나를 레지스트리(정수 키)를 거쳐 직접 넣는다 — 테스트 시드용 (`last_error` 는 `NULL`)
    pub fn seed_task(&self, plugin: &str, provider: &str, asset: i64, target: i64, status: &str, last_error: Option<&str>) {
        self.exec(&format!("INSERT OR IGNORE INTO plugins (plugin_id) VALUES ('{plugin}')"));
        self.exec(&format!("INSERT OR IGNORE INTO plugin_providers (plugin_ref, name) SELECT plugin_ref, '{provider}' FROM plugins WHERE plugin_id = '{plugin}'"));
        let err = last_error.map_or_else(|| "NULL".to_owned(), |e| format!("'{e}'"));
        self.exec(&format!(
            "INSERT INTO plugin_tasks (plugin_ref, provider_ref, asset_id, target_revision, status, last_error, updated_at) \
             SELECT p.plugin_ref, v.provider_ref, {asset}, {target}, '{status}', {err}, 1 FROM plugins p JOIN plugin_providers v USING (plugin_ref) WHERE p.plugin_id = '{plugin}' AND v.name = '{provider}'"
        ));
    }

    /// 플러그인의 모든 작업 행 수
    pub fn task_rows(&self, plugin: &str) -> i64 {
        self.count(&format!("SELECT COUNT(*) FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = '{plugin}'"))
    }

    pub fn pending_tasks(&self, plugin: &str) -> i64 {
        self.count(&format!("SELECT COUNT(*) FROM plugin_tasks JOIN plugins USING (plugin_ref) WHERE plugin_id = '{plugin}' AND status = 'pending'"))
    }

    pub fn jobs_rows(&self) -> Vec<JobRow> {
        self.pool
            .read(|c| {
                let mut st = c.prepare("SELECT asset_id, status, attempts, not_before FROM jobs WHERE kind = 'plugin' ORDER BY job_id")?;
                let rows = st.query_map([], |r| Ok(JobRow { asset: r.get(0)?, status: r.get(1)?, attempts: r.get(2)?, not_before: r.get(3)? }))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap()
    }

    pub fn kv(&self, plugin: &str, asset: i64, key: &str) -> Option<String> {
        let (p, k) = (plugin.to_owned(), key.to_owned());
        self.pool
            .read(move |c| Ok(c.query_row("SELECT value FROM plugin_kv JOIN plugins USING (plugin_ref) WHERE plugin_id=?1 AND asset_id=?2 AND key=?3", (p, asset, k), |r| r.get(0)).ok()))
            .unwrap()
    }

    pub fn kv_rows(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM plugin_kv")
    }

    /// 자산에 붙은 태그 이름 (정렬)
    pub fn tags(&self, asset: i64) -> Vec<String> {
        self.pool
            .read(move |c| {
                let mut st = c.prepare("SELECT t.name FROM asset_tags a JOIN tags t ON t.tag_id = a.tag_id WHERE a.asset_id = ?1 ORDER BY t.name")?;
                let rows = st.query_map([asset], |r| r.get::<_, String>(0))?;
                Ok(rows.collect::<Result<Vec<_>, _>>()?)
            })
            .unwrap()
    }

    /// 자동(플러그인) 태그만 — 사용자 태그(`user-tag-*`)를 뺀다
    pub fn plugin_tags(&self, asset: i64) -> Vec<String> {
        self.tags(asset).into_iter().filter(|t| !t.starts_with("user-tag-")).collect()
    }
}
