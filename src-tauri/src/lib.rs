mod ai;
mod ai_commands;
mod allowed_cache;
mod backup_scheduler;
mod commands;
#[cfg(debug_assertions)]
mod dev_seed;
mod error;
mod exit_run;
mod folder_detection_commands;
mod ingest_commands;
#[cfg(test)]
mod load_soak;
mod locale;
mod logging;
mod metadata_commands;
mod metadata_types;
mod native;
mod pick_save;
mod play_range_commands;
mod player_commands;
mod player_subtitle;
mod player_surface;
mod plugin_commands;
mod plugin_errors;
mod plugin_job_commands;
mod progress;
mod protect_commands;
mod protection;
mod protocols;
mod purge_backup;
mod remote_source_commands;
mod reset;
mod restore;
mod resume_commands;
mod reuse_commands;
mod scene_commands;
mod search_collection_order;
mod search_commands;
mod search_hybrid;
mod search_service;
mod search_user_fields;
mod session_end;
mod shutdown;
mod startup;
mod storage_commands;
mod storage_open;
mod subtitle_commands;
mod system_volume;
mod tray;
mod update;
mod update_guard;
mod user_commands;
mod volume_commands;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use mam_core::ai_jobs::AiJobsConfig;
use mam_core::ingest::{Ingest, IngestConfig, IngestDeps};
use mam_core::jobs::JobKind;
use mam_core::processing::ProcessingEnv;
use mam_core::query::QueryService;
use mam_core::reuse::{Reuse, ShellLauncher, StdFs};
use mam_core::storage::{self, LocationSource, StorageError, SystemDriveProbe, resolve_location};
use mam_core::userdata::UserData;
use mam_core::{Blake3Hasher, WindowsFileIdentity, WindowsVolumes};
use mam_db::{DEFAULT_READERS, DbPool, SystemClock};
use mam_media::{CancellableProcessRunner, MediaTools, SidecarGate, ToolConfig};
use mam_plugin_runtime::{
    GatewayConfig, HostSystemClock, PluginJobs, PluginJobsDeps, PluginRuntime, RuntimeDeps,
};
use tauri::{Manager, RunEvent};

use ai::{AiRunner, AiService, EventSink, LateSink, LocalEmbedders, ModelService, TauriSink};
use backup_scheduler::BackupScheduler;
use protection::{OriginalsDeps, Protection};
use search_service::SearchService;
use storage_commands::Storage;
use update_guard::Paths;

/// `tauri.conf.json` 의 메인 창 제목 — 두 번째 인스턴스가 기존 창을 찾는 데 쓴다.
const MAIN_WINDOW_TITLE: &str = native::APP_DISPLAY_NAME;

fn fatal(text: &str, detail: &std::path::Path) -> ! {
    native::show_error(
        native::ERROR_TITLE,
        &text.replace("{log}", &detail.display().to_string()),
    );
    std::process::exit(1)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    let version = context.package_info().version.to_string();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let verify_mode = args.iter().any(|a| a == startup::VERIFY_FLAG);

    // 1. 위치 결정 -> 2. 디렉터리 생성 -> 3. 로깅 (D6). 여기서 실패하면 오류 창 후 종료
    let bootstrap_dir = storage::bootstrap_dir().unwrap_or_else(|e| {
        native::show_error(native::ERROR_TITLE, &e.to_string());
        std::process::exit(1)
    });
    let resolved = resolve_location(
        std::env::var_os("MAM_DATA_DIR"),
        &bootstrap_dir,
        &SystemDriveProbe,
    );
    let paths = Paths::new(resolved.data_dir.clone());

    // 2-0. 사용자 세션 단위 단일 인스턴스(앱 식별자 뮤텍스) — 디렉터리 생성·로깅·`.lock` 보다 먼저.
    //      위치 결정은 읽기 전용이라 두 번째 인스턴스는 MAMData 에 아무것도 만들거나 쓰지 않고 끝난다.
    //      검증 모드(NSIS 훅)는 잠그지 않는다. 뮤텍스 생성 자체가 실패하면(드묾) `.lock` 이 남아 있으므로 계속한다.
    let mut mutex_error = None;
    let _instance_mutex = if verify_mode {
        None
    } else {
        match reset::acquire_instance_mutex(&paths, &context.config().identifier) {
            Ok(m) => Some(m),
            Err(StorageError::AlreadyRunning) => {
                native::focus_existing_window(MAIN_WINDOW_TITLE);
                std::process::exit(0)
            }
            Err(e) => {
                mutex_error = Some(e);
                None
            }
        }
    };

    if paths.ensure_layout().is_err() {
        fatal(native::LOCATION_FAILED, paths.root());
    }
    logging::init(&paths.logs());
    if !verify_mode {
        locale::ensure_locales_dir(&bootstrap_dir);
    }
    for w in &resolved.warnings {
        tracing::warn!("{w}");
    }
    if let Some(e) = mutex_error {
        tracing::warn!(%e, "cannot create the per-session instance mutex; relying on the data-dir lock");
    }

    // 4. 단일 인스턴스 잠금 — 검증 모드(NSIS 훅)는 잠그지 않는다
    let lock = if verify_mode {
        None
    } else {
        // 초기화 재시작이면 이전 프로세스가 끝날 때까지 잠시 기다린다 (reset.rs)
        match reset::acquire_lock(&paths) {
            Ok(lock) => Some(lock),
            Err(StorageError::AlreadyRunning) => {
                // 창을 만들지 않고 기존 창을 앞으로 가져온 뒤 정상 종료한다
                tracing::info!("another instance is running; focusing its window");
                native::focus_existing_window(MAIN_WINDOW_TITLE);
                std::process::exit(0)
            }
            Err(e) => {
                tracing::error!(%e, "cannot acquire instance lock");
                fatal(native::STARTUP_FAILED, &paths.logs())
            }
        }
    };
    // NSIS 훅이 같은 위치를 보도록 현재 위치를 기록한다 (시험용 환경 변수 재정의는 기록하지 않는다)
    if !verify_mode
        && resolved.source != LocationSource::Env
        && let Err(e) = storage::write_mirror(&bootstrap_dir, paths.root())
    {
        tracing::warn!(%e, "cannot record data directory for the installer");
    }

    // 5. 업데이트 안전장치(journal·DB 복원) -> 마이그레이션 — DB 를 열기 전에 복원이 끝나야 한다
    if let startup::Boot::Exit(code) = startup::boot(
        &args,
        &version,
        &paths,
        &SystemClock,
        &startup::ProcessLauncher,
    ) {
        if code != 0 && !verify_mode {
            native::show_error(
                native::ERROR_TITLE,
                &native::STARTUP_FAILED.replace("{log}", &paths.logs().display().to_string()),
            );
        }
        std::process::exit(code);
    }

    // 6. 연결 풀
    let pool = Arc::new(
        DbPool::open(&paths.db(), DEFAULT_READERS).unwrap_or_else(|e| {
            tracing::error!(%e, "cannot open database pool");
            fatal(native::STARTUP_FAILED, &paths.logs())
        }),
    );
    let locales_seed_dir = locale::locales_dir(&bootstrap_dir);
    let storage = Arc::new(Storage {
        paths: paths.clone(),
        pool,
        bootstrap_dir,
        _lock: lock,
    });
    if let Err(e) = storage.get_settings() {
        tracing::warn!(code = ?e.code(), "cannot read settings at startup");
    }
    tracing::info!(version = %version, "starting");

    // 7. 수집 서비스 (작업 복구 -> 실행기 -> 감시 -> 주기 점검). MAMData·백업 폴더는 스캔에서 제외한다 (PL8)
    let backups_dir = storage
        .app_paths()
        .map_or_else(|_| paths.backups(), |p| PathBuf::from(p.backups_dir));
    let ai_resources = ai::service::default_resources();
    let ai_jobs_cfg = AiJobsConfig::new(paths.cache());
    let ingest = Ingest::new(
        IngestDeps {
            pool: Arc::clone(&storage.pool),
            clock: Arc::new(SystemClock),
            volumes: Arc::new(WindowsVolumes),
            identity: Arc::new(WindowsFileIdentity),
            hasher: Arc::new(Blake3Hasher),
        },
        IngestConfig {
            auto_excludes: vec![paths.root().to_path_buf(), backups_dir.clone()],
            concurrency: ai_jobs_cfg.queue_caps(),
            // 스캔이 방금 수정된(60초 이내) 파일을 반영하지 않고 안정화 게이트에 넘긴다 (결정 I)
            pending: mam_core::ingest::PendingConfig {
                recent_guard_ms: mam_core::ingest::SCAN_RECENT_GUARD_MS,
                ..mam_core::ingest::PendingConfig::default()
            },
            ..IngestConfig::default()
        },
    );
    // 7b. 자산 처리 (P2-3): 메타 -> 썸네일 -> 해시 실행기. FFmpeg 가 없어도 이미지 처리는 동작한다 (기능 저하)
    // 사이드카 게이트: USB "꺼내기 준비" 가 그 디스크 아래 파일을 쓰는 FFmpeg/FFprobe 를 막고 죽인다 (PID 기준 트리 종료)
    let sidecar_gate = SidecarGate::new();
    ingest.set_sidecar_gate(Arc::clone(&sidecar_gate));
    let media_tools = Arc::new(MediaTools::discover(
        &ToolConfig::default().with_exe_sidecar(),
        // 종료 신호가 켜지면 실행 중인 FFmpeg/FFprobe 를 바로 죽인다 (종료가 사이드카 시간 초과를 기다리지 않게)
        Arc::new(
            CancellableProcessRunner::new(ingest.jobs().shutdown_handle()).with_gate(sidecar_gate),
        ),
    ));
    if !(media_tools.has_ffmpeg() && media_tools.has_ffprobe()) {
        tracing::warn!("ffmpeg/ffprobe not found; video and audio processing is degraded");
    }
    let proc_env = ProcessingEnv::new(
        Arc::clone(&storage.pool),
        Arc::new(SystemClock),
        media_tools,
        Arc::new(Blake3Hasher),
        paths.cache(),
    );
    ingest.enable_processing(Arc::clone(&proc_env));
    // 7c. AI (P3): 설정·키·로컬 모델·어댑터·작업 실행기 — `Ingest::start` 전에 실행기를 꽂는다
    let ai_sink = Arc::new(LateSink::default());
    let ai_models = ModelService::new(
        Arc::new(mam_ai::models::ModelStore::new(
            paths.models(),
            mam_ai::models::Manifest::builtin().unwrap_or_else(|e| {
                tracing::error!(%e, "built-in model manifest is invalid");
                fatal(native::STARTUP_FAILED, &paths.logs())
            }),
        )),
        Arc::clone(&ai_sink) as _,
    );
    let ai_embed_store = Arc::new(
        mam_vector::EmbedStore::new(paths.root().join("indexes").join("vectors")).unwrap_or_else(
            |e| {
                tracing::error!(%e, "cannot open the vector store");
                fatal(native::STARTUP_FAILED, &paths.logs())
            },
        ),
    );
    let ai = AiService::new(ai::AiServiceDeps {
        pool: Arc::clone(&storage.pool),
        clock: Arc::new(SystemClock),
        queue: Arc::clone(ingest.jobs()),
        transport: Arc::new(mam_ai::external::UreqTransport::new()),
        embedders: Arc::new(LocalEmbedders::new(
            Arc::clone(ai_models.store()),
            ai_resources.embed_config(),
        )),
        models: ai_models,
        embed_store: ai_embed_store,
        sink: Arc::clone(&ai_sink) as _,
        stt_runner: Arc::new(mam_ai::local::stt::ProcessRunner),
        stt_tools: Arc::new(ai::system_tools),
        stt_threads: ai_resources.stt_config().threads,
        data_root: paths.root().to_path_buf(),
        jobs_cfg: ai_jobs_cfg,
        utc_offset: Arc::new(ai::local_utc_offset_min),
        cli_runner: Arc::new(mam_ai::external::cli::OsRunner),
        cli_env: Arc::new(mam_ai::external::cli::SystemEnv),
        url_opener: Arc::new(ai::ShellUrlOpener),
    })
    .unwrap_or_else(|e| {
        tracing::error!(?e, "cannot start AI service");
        fatal(native::STARTUP_FAILED, &paths.logs())
    });
    ai.load_persisted_keys();
    ingest.register_executor(JobKind::Ai, Arc::new(ai.make_executor()));
    // 7d. 장면전환점 검출 — 커밋 뒤 `scene://updated { assetId }` (setup 전 이벤트는 LateSink 가 버린다)
    let scene_sink = Arc::clone(&ai_sink);
    ingest.register_executor(
        JobKind::Scene,
        Arc::new(mam_core::scene::SceneExecutor::new(
            proc_env,
            Arc::new(move |asset_id| {
                let payload = mam_core::scene::SceneUpdatedPayload { asset_id };
                match serde_json::to_value(payload) {
                    Ok(v) => scene_sink.emit(mam_core::scene::SCENE_UPDATED_EVENT, v),
                    Err(e) => tracing::debug!(%e, "cannot serialize scene event"),
                }
            }),
        )),
    );
    // 7d-2. 플러그인 런타임(P6)·작업(P5): 부팅은 실패하지 않는다(깨진 플러그인은 목록에 `broken`). 실행기는 `Ingest::start` 전에,
    //       변경 피드 스레드는 그 뒤에 시작한다. `plugin://changed` 는 LateSink 로 보낸다(setup 전 이벤트는 버려진다)
    let plugin_sink = Arc::clone(&ai_sink);
    let plugin_runtime = PluginRuntime::boot(RuntimeDeps {
        pool: Arc::clone(&storage.pool),
        clock: Arc::new(SystemClock),
        host_clock: Arc::new(HostSystemClock),
        data_root: paths.root().to_path_buf(),
        plugins_dir: paths.plugins(),
        worker_exe: mam_plugin_runtime::locate_worker_default(),
        app_version: semver::Version::parse(&version)
            .unwrap_or_else(|_| semver::Version::new(0, 0, 0)),
        gateway: GatewayConfig::new(paths.cache()),
        on_change: Arc::new(move |change| match serde_json::to_value(&change) {
            Ok(v) => plugin_sink.emit(mam_plugin_runtime::PLUGIN_CHANGED_EVENT, v),
            Err(e) => tracing::debug!(%e, "cannot serialize plugin event"),
        }),
    });
    let plugin_jobs = PluginJobs::new(PluginJobsDeps {
        pool: Arc::clone(&storage.pool),
        clock: Arc::new(SystemClock),
        queue: Arc::clone(ingest.jobs()),
        invoker: Arc::clone(&plugin_runtime) as _,
        gateway: plugin_runtime.gateway(),
        protected: vec![paths.root().to_path_buf(), backups_dir.clone()],
    });
    // 백업 폴더는 실행 중 설정으로 바뀔 수 있다 — 내보내기마다 현재 설정으로 다시 계산한다(고정 목록은 부팅 시점 값)
    plugin_jobs.set_protected_source(plugin_job_commands::plugin_protected_source(
        Arc::clone(&storage),
        paths.root().to_path_buf(),
    ));
    plugin_runtime.set_job_hooks(Arc::clone(&plugin_jobs) as _);
    ingest.register_executor(JobKind::Plugin, plugin_jobs.executor());
    // 7e. 새 디스크 감지: 대기 목록 이벤트 + 장치 변경 알림(숨긴 최상위 창)으로 점검을 즉시 깨운다 (1초 디바운스)
    volume_commands::wire_pending_events(&ingest, Arc::clone(&ai_sink) as _);
    ingest.set_device_watcher(
        Arc::new(mam_core::WindowsDeviceWatcher),
        std::time::Duration::from_secs(1),
    );
    // 7f. 연결 데이터 삭제(폴더 제거·유형 정리·옛 보관 데이터 정리): 직전 DB 백업, 캐시 정리 위치, 진행 이벤트 - `start` 전에 연결
    ingest.set_backup_hook(purge_backup::backup_hook(
        Arc::clone(&storage),
        Arc::new(SystemClock),
    ));
    ingest.set_cache_root(paths.cache());
    // 지운 임베딩의 모델 키를 벡터 저장소에 알려 활성 맵 캐시가 지운 슬롯을 바로 버리게 한다
    {
        let store = Arc::clone(&ai.embed_store);
        ingest.set_embeddings_changed_hook(Arc::new(move |keys| {
            for key in keys {
                store.notify_changed(key);
            }
        }));
    }
    volume_commands::wire_removal_events(&ingest, Arc::clone(&ai_sink) as _);
    let protocol_cache = paths.cache();
    let protocol_pool = Arc::clone(&storage.pool);
    if let Err(e) = ingest.start() {
        tracing::error!(%e, "cannot start ingest service");
        fatal(native::STARTUP_FAILED, &paths.logs())
    }
    plugin_jobs.start_feed();
    #[cfg(debug_assertions)]
    dev_seed::spawn_from_env(&ingest);
    // 8. 사용자 데이터(P2-7)·재활용(P2-8). 내보내기 대상으로는 관리 폴더, MAMData, 백업 폴더를 금지한다
    let user_data = Arc::new(UserData::new(
        Arc::clone(&storage.pool),
        Arc::new(SystemClock),
    ));
    // 8a. 이전 실행에서 끝내지 못한 사용자 필드·선택지 청크 삭제를 이어서 한다 (UI 와 무관한 전용 스레드,
    //     청크마다 짧은 트랜잭션 + 양보). 삭제 요청 직후 실행은 요청 command 가 추가될 때 같은 함수를 부른다.
    {
        let user_data = Arc::clone(&user_data);
        if let Err(e) = std::thread::Builder::new()
            .name("mam-umeta-purge".into())
            .spawn(move || {
                if let Err(e) = user_data.purge_pending_metadata_deletions() {
                    tracing::warn!(%e, "custom metadata purge failed; it resumes on the next start");
                }
            })
        {
            tracing::warn!(%e, "cannot start the custom metadata purge");
        }
    }
    let reuse = Arc::new(Reuse::new(
        Arc::clone(&storage.pool),
        Arc::new(SystemClock),
        Arc::new(StdFs),
        Arc::new(ShellLauncher),
        vec![paths.root().to_path_buf(), backups_dir],
    ));
    // 8b. 검색(P2-5): 조회 서비스 + 색인 워커 스레드(시작 시 필요하면 SQLite 에서 재구축)
    let query = Arc::new(QueryService::new(
        Arc::clone(&storage.pool),
        Arc::new(SystemClock),
    ));
    let search = SearchService::start(
        Arc::clone(&storage.pool),
        Arc::new(SystemClock),
        paths.tantivy(),
        Arc::clone(&query),
    );
    let ai_runner = AiRunner::start(Arc::clone(&ai));
    let query_embedder = Arc::new(ai::LocalQueryEmbedder::new(
        Arc::clone(&ai.embedders),
        Arc::clone(&ai.models),
        Arc::clone(&ai.store),
    ));
    search.set_query_embedder(query_embedder.clone());
    // 첫 화면이 그려진 뒤 질의 모델(e5)을 미리 읽는다 — 켜져 있지 않으면 아무 일도 하지 않는다
    if let Err(e) = std::thread::Builder::new()
        .name("mam-e5-warmup".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            query_embedder.warm_up();
        })
    {
        tracing::warn!(%e, "cannot start the query model warm-up");
    }
    search.set_embed_store(Arc::clone(&ai.embed_store));
    // 활성 임베딩 맵(100만 벡터 약 1초)도 같은 시점에 백그라운드에서 읽어 둔다 — 첫 하이브리드/유사 질의가 느리지 않게
    let warm_search = Arc::clone(&search);
    if let Err(e) = std::thread::Builder::new()
        .name("mam-vecmap-warmup".into())
        .spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            warm_search.warm_vector_maps();
            warm_search.warm_allowed_set(std::time::Duration::from_secs(300));
        })
    {
        tracing::warn!(%e, "cannot start the vector map warm-up");
    }
    let progress_stop = Arc::new(AtomicBool::new(false));
    // 9. 보호(P2-9): 비밀번호·원본 삭제/이동·초기화, 자동 백업 타이머 (앱 실행 중에만)
    let protection = Arc::new(Protection::new(
        Arc::clone(&storage),
        Arc::clone(&ingest),
        Arc::new(SystemClock),
        OriginalsDeps::system(),
    ));
    let scheduler = BackupScheduler::start(
        Arc::clone(&storage),
        Arc::new(SystemClock),
        backup_scheduler::FIRST_DELAY,
        backup_scheduler::TICK,
    )
    .unwrap_or_else(|e| {
        tracing::error!(%e, "cannot start auto backup scheduler");
        fatal(native::STARTUP_FAILED, &paths.logs())
    });

    let healthy_paths = paths.clone();
    let exit_services = Arc::new(exit_run::ExitServices {
        progress_stop: Arc::clone(&progress_stop),
        scheduler: Arc::clone(&scheduler),
        search: Arc::clone(&search),
        ai_runner: Arc::clone(&ai_runner),
        ingest: Arc::clone(&ingest),
        ai_requests: ai.in_flight(),
        ai_service: Arc::clone(&ai),
        plugin_jobs: Arc::clone(&plugin_jobs),
        plugin_runtime: Arc::clone(&plugin_runtime),
        storage: Arc::clone(&storage),
        once: exit_run::RunOnce::default(),
    });
    let setup_exit = Arc::clone(&exit_services);
    let setup_sink = Arc::clone(&ai_sink);
    let setup_ingest = Arc::clone(&ingest);
    let tray_ingest = Arc::clone(&ingest);
    let eject_ingest = Arc::clone(&ingest);
    let window_storage = Arc::clone(&storage);
    let setup_stop = Arc::clone(&progress_stop);
    let player_cache = protocol_cache.clone();
    let built = protocols::register(tauri::Builder::default(), protocol_cache, protocol_pool)
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(paths)
        .manage(storage)
        .manage(ingest)
        .manage(user_data)
        .manage(reuse)
        .manage(Arc::new(subtitle_commands::SubtitleService::default()))
        .manage(query)
        .manage(search)
        .manage(ai)
        .manage(protection)
        .manage(plugin_runtime)
        .manage(plugin_jobs)
        .setup(move |app| {
            // 동봉 언어팩 첫 실행 복사 — 없는 파일만, 실패해도 시작을 막지 않는다 (검증 모드는 setup 에 오지 않는다)
            match app.path().resource_dir() {
                Ok(res) => {
                    let report = locale::seed::seed_bundled_locales(&res.join(locale::LOCALES_DIR_NAME), &locales_seed_dir);
                    if !report.copied.is_empty() {
                        tracing::info!(count = report.copied.len(), "seeded bundled language packs");
                    }
                }
                Err(e) => tracing::warn!(%e, "cannot resolve the resource dir; skipping bundled language packs"),
            }
            progress::spawn(app.handle().clone(), setup_ingest, setup_stop);
            // Windows 종료·재시작·로그오프: 트레이에 상주한 채 세션이 끝나도 종료 정리(표식 삭제)를 실행한다
            #[cfg(windows)]
            install_session_end_hook(app, &setup_exit);
            setup_sink.set(Arc::new(TauriSink(app.handle().clone())));
            // 내장 플레이어(libmpv 워커) - 여기서는 위치만 찾고, 워커는 첫 `player_open` 에서 띄운다
            let player = Arc::new(player_commands::PlayerService::new(app.handle(), player_cache));
            // 디스크 꺼내기 준비: 그 디스크의 파일을 재생 중이면 정지한다 (워커가 파일을 쥐고 있다)
            let eject_player = Arc::clone(&player);
            eject_ingest.set_eject_hook(Arc::new(move |scope| eject_player.close_if_on_volume(scope)));
            app.manage(player);
            // 트레이를 못 만들어도 앱은 뜬다 (창 닫기가 종료로 동작하도록 closeToTray 가 있어도 숨김은 트레이 전제)
            if let Err(e) = tray::install(app.handle(), tray_ingest) {
                tracing::warn!(%e, "cannot create tray icon");
            }
            Ok(())
        })
        // 페이지가 다시 로드되면(F5·개발 서버 새로고침·WebView 복구) 화면 상태는 초기화되므로 살아 있는 플레이어·영상 창을 정리한다
        .on_page_load(|webview, payload| {
            if webview.label() == "main" && matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                player_commands::on_page_load_started(webview.app_handle());
            }
        })
        .on_window_event(move |window, event| {
            tray::on_window_event(window, event, &window_storage);
            player_commands::on_window_event(window, event);
        })
        .manage(update::PendingUpdate::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            update::check_update,
            update::install_update,
            storage_commands::get_settings,
            storage_commands::update_settings,
            storage_commands::backup_now,
            storage_commands::list_backups,
            storage_commands::app_paths,
            storage_open::open_backup_dir,
            locale::commands::list_locales,
            locale::commands::read_locale,
            locale::commands::open_locale_folder,
            restore::commands::restore_backup,
            restore::commands::take_startup_notice,
            ingest_commands::list_folders,
            ingest_commands::add_folder,
            ingest_commands::relocate_folder,
            ingest_commands::remove_folder,
            ingest_commands::folder_removal_impact,
            ingest_commands::rescan_folder,
            ingest_commands::set_folder_hidden,
            ingest_commands::set_folder_alias,
            ingest_commands::set_folder_media_types,
            folder_detection_commands::list_change_detectors,
            folder_detection_commands::set_folder_watch_mode,
            folder_detection_commands::set_folder_rescan_policy,
            remote_source_commands::list_remote_sources,
            remote_source_commands::create_remote_source,
            remote_source_commands::update_remote_source,
            remote_source_commands::delete_remote_source,
            remote_source_commands::test_remote_source,
            ingest_commands::count_folder_assets_by_type,
            ingest_commands::list_subfolders,
            ingest_commands::job_summary,
            ingest_commands::pause_jobs,
            ingest_commands::resume_jobs,
            ingest_commands::list_scan_errors,
            ingest_commands::retry_scan_errors,
            ingest_commands::pick_directory,
            pick_save::pick_save_file,
            volume_commands::list_pending_new_volumes,
            volume_commands::dismiss_new_volume,
            volume_commands::list_ignored_volumes,
            volume_commands::unignore_volume,
            volume_commands::list_known_volumes,
            volume_commands::forget_volume_record,
            volume_commands::prepare_volume_eject,
            volume_commands::resume_volume_watch,
            user_commands::list_tags,
            user_commands::update_asset_tags,
            user_commands::rename_tag,
            user_commands::delete_tag,
            user_commands::set_memo,
            user_commands::set_asset_title,
            user_commands::set_favorite,
            user_commands::set_rating,
            user_commands::rating_counts,
            user_commands::list_collections,
            user_commands::create_collection,
            user_commands::rename_collection,
            user_commands::delete_collection,
            user_commands::add_to_collection,
            user_commands::remove_from_collection,
            user_commands::reorder_collection,
            user_commands::collection_membership,
            user_commands::collection_play_queue,
            metadata_commands::list_metadata_fields,
            metadata_commands::create_metadata_field,
            metadata_commands::rename_metadata_field,
            metadata_commands::reorder_metadata_fields,
            metadata_commands::delete_metadata_field,
            metadata_commands::metadata_field_value_count,
            metadata_commands::add_metadata_option,
            metadata_commands::rename_metadata_option,
            metadata_commands::reorder_metadata_options,
            metadata_commands::delete_metadata_option,
            metadata_commands::metadata_option_usage,
            metadata_commands::get_asset_metadata,
            metadata_commands::set_asset_metadata,
            metadata_commands::bulk_set_asset_metadata,
            play_range_commands::play_ranges_get,
            play_range_commands::play_ranges_set,
            play_range_commands::collection_play_range_summary,
            resume_commands::resume_get,
            resume_commands::resume_save,
            resume_commands::resume_count_assets,
            resume_commands::resume_clear_assets,
            resume_commands::resume_clear_folder,
            resume_commands::resume_clear_all,
            resume_commands::resume_set_folder_enabled,
            resume_commands::resume_stats,
            scene_commands::scene_cuts_get,
            scene_commands::scene_cut_add,
            scene_commands::scene_cut_remove,
            scene_commands::scene_cuts_clear_auto,
            scene_commands::scene_detect_request,
            scene_commands::scene_detect_backfill,
            scene_commands::scene_backfill_count,
            reuse_commands::export_assets,
            reuse_commands::copy_to_clipboard,
            reuse_commands::open_original,
            reuse_commands::reveal_in_explorer,
            protect_commands::get_protection_status,
            protect_commands::set_password,
            protect_commands::change_password,
            protect_commands::verify_password,
            protect_commands::delete_originals,
            protect_commands::move_originals,
            protect_commands::reset_managed_data,
            search_commands::search_assets,
            search_commands::search_index,
            search_commands::find_similar,
            search_commands::get_asset_detail,
            search_commands::list_recent_searches,
            search_commands::record_recent_search,
            search_commands::clear_recent_searches,
            ai_commands::get_ai_settings,
            ai_commands::update_ai_settings,
            ai_commands::give_ai_consent,
            ai_commands::revoke_ai_consent,
            ai_commands::list_ai_providers,
            ai_commands::set_api_key,
            ai_commands::clear_api_key,
            ai_commands::set_provider_model,
            ai_commands::check_provider,
            ai_commands::set_provider_auth_mode,
            ai_commands::detect_ai_cli,
            ai_commands::ai_cli_login_start,
            ai_commands::ai_cli_login_status,
            ai_commands::ai_cli_login_cancel,
            ai_commands::open_ai_cli_login_url,
            ai_commands::ai_cli_logout,
            ai_commands::list_local_models,
            ai_commands::download_model,
            ai_commands::analyze_assets,
            ai_commands::analyze_all,
            ai_commands::generate_subtitles_all,
            ai_commands::retry_failed_analysis,
            ai_commands::get_asset_ai,
            ai_commands::update_analysis,
            ai_commands::list_analysis_history,
            ai_commands::list_ai_usage,
            player_commands::player_available,
            player_commands::player_open,
            player_commands::player_control,
            player_commands::player_seek,
            player_commands::player_set_rate,
            player_commands::player_set_volume,
            player_commands::player_set_render_size,
            player_commands::player_set_render_active,
            player_commands::player_release_slots,
            player_commands::player_restart,
            player_commands::player_close,
            plugin_commands::plugin_list,
            plugin_commands::plugin_inspect_dir,
            plugin_commands::plugin_install,
            plugin_commands::plugin_set_enabled,
            plugin_commands::plugin_grant_permissions,
            plugin_commands::plugin_uninstall,
            plugin_commands::plugin_run_command,
            plugin_commands::plugin_contributions,
            plugin_job_commands::plugin_run_all,
            plugin_job_commands::plugin_export,
            plugin_job_commands::plugin_task_counts,
            plugin_job_commands::plugin_retry_failed,
            player_commands::player_set_ranges,
            subtitle_commands::subtitle_tracks,
            subtitle_commands::subtitle_cues,
            subtitle_commands::player_set_subtitle,
            system_volume::system_volume_step,
            tray::hide_main_window,
            tray::set_tray_labels
        ])
        .build(context);
    let app = match built {
        Ok(app) => app,
        Err(e) => {
            tracing::error!(%e, "cannot build application");
            fatal(native::STARTUP_FAILED, &healthy_paths.logs())
        }
    };
    app.run(move |app, event| match event {
        // 이벤트 루프가 돌고 창이 만들어진 시점 = 정상 기동 (D4)
        RunEvent::Ready => {
            if let Err(e) = update_guard::mark_healthy(&healthy_paths, &SystemClock) {
                tracing::warn!(%e, "cannot record healthy start");
            }
        }
        // 정리 본문은 exit_run — Windows 세션 종료 경로(session_end)와 공유하며 한 번만 실행된다
        RunEvent::Exit => {
            let mode = if session_end::is_ending() {
                exit_run::Mode::SessionEnd
            } else {
                exit_run::Mode::Normal
            };
            exit_services.run(app, mode);
        }
        _ => {}
    });
}

/// 메인 창에 세션 종료 처리기를 건다. 실패해도 시작을 막지 않는다 (tao 의 `WM_ENDSESSION` → `Exit` 경로가 남는다).
#[cfg(windows)]
fn install_session_end_hook(app: &tauri::App, services: &Arc<exit_run::ExitServices>) {
    let Some(window) = app.get_webview_window("main") else {
        tracing::warn!("no main window; session end hook not installed");
        return;
    };
    let Ok(hwnd) = window.hwnd() else {
        tracing::warn!("cannot get the main window handle; session end hook not installed");
        return;
    };
    let (handle, services) = (app.handle().clone(), Arc::clone(services));
    let installed = session_end::install(
        hwnd.0,
        Arc::new(move || {
            services.run(&handle, exit_run::Mode::SessionEnd);
        }),
    );
    if !installed {
        tracing::warn!("cannot install the session end hook");
    }
}
