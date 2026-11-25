//! 전체 실행·내보내기 시험 도우미 — 대량 자산, 관리 폴더 등록, 내보내기 플러그인, 정션·8.3 이름, 큐 condvar 대기.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use mam_db::Priority;
use mam_plugin_runtime::jobs::ExportError;
use mam_plugin_runtime::{AssetKind, ExportResult, ProviderKind};

use super::{Env, Plugin};

pub const EXPORTER: &str = "com.example.exporter";

/// 내보내기(`exp.step` 스크립트 / `exp.echo` 자산 목록 반향) + 메타데이터(`meta.run`) provider 를 가진 플러그인
pub fn exporter() -> Plugin {
    Plugin::new(EXPORTER, "exporter")
        .provider("exp.step", ProviderKind::Export, &[AssetKind::Image])
        .provider("exp.echo", ProviderKind::Export, &[AssetKind::Image])
        .provider("meta.run", ProviderKind::Metadata, &[AssetKind::Image])
}

/// 제거 시 지우는 디렉터리 정션 (대상 폴더는 건드리지 않는다)
pub struct Junction(PathBuf);

impl Drop for Junction {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

impl Env {
    /// 활성 자산을 한 트랜잭션으로 `n` 개 만든다 (파일 없음 — 전체 실행처럼 DB 만 보는 시험용). 자산 id 오름차순
    pub fn bulk_assets(&self, n: usize, media: &str) -> Vec<i64> {
        let media = media.to_owned();
        self.pool
            .write(Priority::High, move |c| {
                let tx = c.transaction()?;
                let mut ids = Vec::with_capacity(n);
                {
                    let mut st = tx.prepare("INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at) VALUES (?1, 1, 1, 1, 1)")?;
                    for _ in 0..n {
                        st.execute([&media])?;
                        ids.push(tx.last_insert_rowid());
                    }
                }
                tx.commit()?;
                Ok(ids)
            })
            .unwrap()
    }

    /// `root` 를 관리 폴더로 등록한다 (`FolderRow::root_path` = 볼륨 마운트 지점 + `rel_root` 가 `root` 가 되게)
    pub fn add_managed_root(&self, root: &Path) {
        let (mount, rel) = (
            root.parent().unwrap().to_string_lossy().into_owned(),
            root.file_name().unwrap().to_string_lossy().into_owned(),
        );
        let display = root.to_string_lossy().into_owned();
        self.pool
            .write(Priority::High, move |c| {
                c.execute("INSERT INTO volumes (kind, volume_key, label, last_mount_point, last_seen_at) VALUES ('fixed', ?1, 'M', ?2, 1)", (format!("vol-{display}"), &mount))?;
                let vol = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO managed_folders (volume_id, rel_root, path_key, display_path, created_at) VALUES (?1, ?2, ?3, ?4, 1)",
                    (vol, &rel, mam_db::path_key(&rel).unwrap(), &display),
                )?;
                Ok(())
            })
            .unwrap();
    }

    /// 내보내기 대상으로 쓸 수 있는 폴더 (관리 폴더·MAMData·백업과 겹치지 않는다)
    pub fn out_dir(&self) -> PathBuf {
        let dir = self.tmp.path().join("Out");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    pub fn export(
        &self,
        plugin: &str,
        provider: &str,
        assets: &[i64],
        dest: &Path,
    ) -> Result<ExportResult, ExportError> {
        self.jobs
            .export(plugin, provider, assets.to_vec(), &dest.to_string_lossy())
    }

    /// 큐 condvar 로만 기다린다 (sleep 폴링 없음). 30 초 안에 조건이 안 되면 실패
    pub fn wait_until(&self, what: &str, cond: impl Fn(&Env) -> bool) {
        let (limit, mut seen) = (Instant::now() + Duration::from_secs(30), 0u64);
        while !cond(self) {
            assert!(Instant::now() < limit, "timed out waiting for: {what}");
            self.queue.wait(&mut seen, Duration::from_millis(100));
        }
    }

    pub fn plugin_jobs(&self) -> i64 {
        self.count("SELECT COUNT(*) FROM jobs WHERE kind = 'plugin'")
    }

    /// 자산 `asset` 의 `plugin` 작업 `jobs` 행이 들어가는 트랜잭션에서 그 플러그인의 설정 행을 함께 지운다 —
    /// "제거가 두 배치 사이에 커밋됐다" 를 대기 없이 결정적으로 만든다
    pub fn uninstall_when_job_inserted(&self, plugin: &str, asset: i64) {
        self.exec(&format!(
            "CREATE TRIGGER t_uninstall AFTER INSERT ON jobs WHEN NEW.kind = 'plugin' AND NEW.asset_id = {asset} BEGIN
               DELETE FROM settings WHERE key IN ('plugin_installed:{plugin}', 'plugin_enabled:{plugin}', 'plugin_granted:{plugin}', 'plugin_feed_cursor:{plugin}');
             END"
        ));
    }
}

fn run_cmd(args: String) -> Option<String> {
    let out = Command::new("cmd").raw_arg(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// `mklink /J` — 실패하면 `None` (시험이 이유를 밝히고 건너뛴다)
pub fn junction(link: &Path, target: &Path) -> Option<Junction> {
    run_cmd(format!(
        r#"/C mklink /J "{}" "{}""#,
        link.display(),
        target.display()
    ))?;
    link.is_dir().then(|| Junction(link.to_path_buf()))
}

/// 8.3 짧은 이름 경로 — 볼륨에 짧은 이름이 없으면 원래 경로와 같다 (`None` = 조회 실패)
pub fn short_path(p: &Path) -> Option<PathBuf> {
    let s = run_cmd(format!(r#"/C for %I in ("{}") do @echo %~sI"#, p.display()))?;
    (!s.is_empty()).then(|| PathBuf::from(s))
}
