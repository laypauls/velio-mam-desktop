//! 게이트웨이 시험 공용 — 실제 SQLite(마이그레이션 적용)·임시 폴더 픽스처·기록용 로그 sink.
//! 원본 파일은 임시 폴더에 만든다(실제 사용자 폴더 접근 없음). 시계는 고정한다.
#![allow(dead_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mam_db::plugin_settings::{ENABLED, GRANTED, put};
use mam_db::{DbPool, Priority, migrate, open};
use mam_plugin_gateway::{Gateway, GatewayConfig, LogSink};
use mam_plugin_host::{CallCx, HostApi, HostError, InvokeScope, LogLevel};
use mam_plugin_manifest::{Permission, PluginId, parse_manifest};
use rusqlite::Connection;
use serde_json::{Value, json};
use tempfile::TempDir;

pub const T0: i64 = 1_700_000_000_000;
pub const ALL_PERMS: [Permission; 6] = Permission::ALL;

pub struct FixedClock(pub i64);

impl mam_db::Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

#[derive(Default)]
pub struct Rec(pub Mutex<Vec<(String, LogLevel, String)>>);

impl LogSink for Rec {
    fn emit(&self, plugin_id: &str, level: LogLevel, message: &str) {
        self.0
            .lock()
            .unwrap()
            .push((plugin_id.to_owned(), level, message.to_owned()));
    }
}

impl Rec {
    pub fn lines(&self) -> Vec<(String, LogLevel, String)> {
        self.0.lock().unwrap().clone()
    }
}

pub fn plugin_id(id: &str) -> PluginId {
    let json = format!(
        r#"{{"apiVersion":1,"id":"{id}","version":"0.1.0","engines":{{"mam":">=0.1.0"}},"main":"main.js","displayName":"plugin.name"}}"#
    );
    parse_manifest(&json).expect("valid manifest").id
}

pub struct Fx {
    pub dir: TempDir,
    pub db_path: PathBuf,
    pub pool: Arc<DbPool>,
    pub gw: Arc<Gateway>,
    pub lib: PathBuf,
    pub cache: PathBuf,
    pub sink: Arc<Rec>,
    vol: i64,
}

pub fn soon() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

impl Fx {
    pub fn new() -> Self {
        Self::with_cfg(|_| {})
    }

    pub fn with_cfg(tweak: impl FnOnce(&mut GatewayConfig)) -> Self {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("mam.sqlite");
        let (lib, cache) = (dir.path().join("SecretLibrary"), dir.path().join("cache"));
        std::fs::create_dir_all(&lib).unwrap();
        let mut conn = open(&db_path).unwrap();
        migrate(&mut conn, &dir.path().join("backups"), &FixedClock(T0)).unwrap();
        drop(conn);
        let pool = Arc::new(DbPool::open(&db_path, 2).unwrap());
        let vol = pool
            .write(Priority::High, |c| {
                c.execute("INSERT INTO volumes (kind, volume_key, label, last_mount_point, last_seen_at) VALUES ('fixed', 'vol-x', 'SecretVolumeLabel', 'Z:\\SecretMount', 1)", [])?;
                Ok(c.last_insert_rowid())
            })
            .unwrap();
        let mut cfg = GatewayConfig::new(cache.clone());
        tweak(&mut cfg);
        let sink = Arc::new(Rec::default());
        let gw = Arc::new(
            Gateway::new(Arc::clone(&pool), Arc::new(FixedClock(T0)), cfg)
                .with_log_sink(sink.clone()),
        );
        Self {
            dir,
            db_path,
            pool,
            gw,
            lib,
            cache,
            sink,
            vol,
        }
    }

    /// 원본 파일(임시 폴더 안)과 활성 위치가 있는 자산. 내용은 id 로 달라진다
    pub fn add_asset(&self, media: &str, rel: &str) -> i64 {
        let path = rel.split('/').fold(self.lib.clone(), |p, s| p.join(s));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("original bytes of {rel}")).unwrap();
        let (media, rel, vol) = (media.to_owned(), rel.to_owned(), self.vol);
        self.pool
            .write(Priority::High, move |c| {
                c.execute("INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at) VALUES (?1, 1, 1, 1, 1)", [&media])?;
                let id = c.last_insert_rowid();
                c.execute(
                    "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at) VALUES (?1, ?2, ?3, ?4, 1)",
                    (id, vol, &rel, mam_db::path_key(&rel).unwrap()),
                )?;
                Ok(id)
            })
            .unwrap()
    }

    pub fn exec(&self, sql: &str) {
        let sql = sql.to_owned();
        self.pool
            .write(Priority::High, move |c| Ok(c.execute_batch(&sql)?))
            .unwrap();
    }

    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> T) -> T {
        self.pool.read(|c| Ok(f(c))).unwrap()
    }

    pub fn count(&self, sql: &str) -> i64 {
        self.read(|c| c.query_row(sql, [], |r| r.get(0)).unwrap())
    }

    /// 실제 WebP 썸네일(320·1024)을 캐시에 만든다. 원본 PNG 는 임시 폴더에만 둔다
    pub fn thumbs(&self, id: i64) {
        let src = self.dir.path().join(format!("thumb_src_{id}.png"));
        image::RgbImage::from_fn(1500, 900, |x, _| {
            if x < 750 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 0, 255])
            }
        })
        .save(&src)
        .unwrap();
        let tools = mam_media::tool::MediaTools::with_paths(
            None,
            None,
            Arc::new(mam_media::tool::ProcessRunner),
        );
        mam_media::thumbnail::create_thumbnails(
            &tools,
            &src,
            id,
            &self.cache,
            &mam_media::thumbnail::ThumbnailOptions::default(),
        )
        .unwrap();
    }

    /// 플러그인을 등록하고 허용 상태(`enabled`, `granted` 원문)를 DB 에 쓴 뒤 `refresh` 한다
    pub fn plugin(
        &self,
        id: &str,
        manifest: &[Permission],
        granted_raw: Option<&str>,
        enabled_raw: Option<&str>,
    ) -> PluginId {
        let pid = plugin_id(id);
        self.gw
            .grants()
            .register(&pid, &manifest.iter().copied().collect::<BTreeSet<_>>());
        self.set_settings(id, enabled_raw, granted_raw);
        self.gw.grants().refresh(&self.pool, &pid, soon()).unwrap();
        pid
    }

    pub fn set_settings(&self, id: &str, enabled_raw: Option<&str>, granted_raw: Option<&str>) {
        let (id, e, g) = (
            id.to_owned(),
            enabled_raw.map(str::to_owned),
            granted_raw.map(str::to_owned),
        );
        self.pool
            .write(Priority::High, move |c| {
                c.execute(
                    "DELETE FROM settings WHERE key IN (?1, ?2)",
                    (
                        format!("plugin_enabled:{id}"),
                        format!("plugin_granted:{id}"),
                    ),
                )?;
                if let Some(e) = e {
                    put(c, ENABLED, &id, &e).unwrap();
                }
                if let Some(g) = g {
                    put(c, GRANTED, &id, &g).unwrap();
                }
                Ok(())
            })
            .unwrap();
    }

    /// 모든 권한 허용 + 활성
    pub fn full_plugin(&self, id: &str) -> PluginId {
        self.plugin(
            id,
            &ALL_PERMS,
            Some(&granted_json(&ALL_PERMS)),
            Some("true"),
        )
    }

    pub fn call(
        &self,
        plugin: &PluginId,
        assets: &[i64],
        method: &str,
        params: Value,
    ) -> Result<Value, HostError> {
        let scope = InvokeScope::new(assets.to_vec())
            .unwrap()
            .with_deadline(soon());
        self.call_scope(plugin, &scope, method, params)
    }

    pub fn call_scope(
        &self,
        plugin: &PluginId,
        scope: &InvokeScope,
        method: &str,
        params: Value,
    ) -> Result<Value, HostError> {
        self.gw.call(&CallCx { plugin, scope }, method, params)
    }

    pub fn err_code(r: Result<Value, HostError>) -> String {
        r.expect_err("expected an error").code
    }
}

pub fn granted_json(perms: &[Permission]) -> String {
    let v: Vec<&str> = perms.iter().map(|p| p.as_str()).collect();
    json!(v).to_string()
}

/// 폴더 아래 모든 파일의 (상대 경로, 내용, 수정 시각) — 원본 보호 단언용
pub fn snapshot(root: &Path) -> Vec<(String, Vec<u8>, std::time::SystemTime)> {
    let mut out = Vec::new();
    for e in walk(root) {
        let m = std::fs::metadata(&e).unwrap();
        out.push((
            e.strip_prefix(root).unwrap().to_string_lossy().into_owned(),
            std::fs::read(&e).unwrap(),
            m.modified().unwrap(),
        ));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            v.extend(walk(&p))
        } else {
            v.push(p)
        }
    }
    v
}
