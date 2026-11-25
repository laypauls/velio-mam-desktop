# DEVELOPER

로컬 MAM 데스크톱 앱(velio.) 개발 환경·명령 안내. 개발 계획은 [docs/plan/development_plan.md](docs/plan/development_plan.md), PoC·성능 측정 절차는 [docs/dev/measurements.md](docs/dev/measurements.md). 설계 근거·이력은 `docs/requests/` 의 각 작업 문서에 있다.

## 필수 도구 (Windows 10/11)

| 도구 | 확인된 버전 | 비고 |
|---|---|---|
| Node.js | 24.x | |
| pnpm | 10.x | 패키지 매니저는 pnpm 만 (`pnpm-lock.yaml`) |
| Rust | stable 1.94+ (`x86_64-pc-windows-msvc`) | `rustup default stable-msvc` |
| Visual Studio 2022 Build Tools | "C++를 사용한 데스크톱 개발" | MSVC 링커, libwebp C 소스 컴파일 |
| WebView2 Runtime | Windows 11 기본 포함 | |

- Tauri CLI 는 전역 설치하지 않는다 — devDependency `@tauri-apps/cli` 를 `pnpm tauri` 로 쓴다
- 첫 빌드는 네트워크가 필요하다: mecab-ko-dic(약 48MB, `lindera-ko-dic` build.rs → 바이너리 임베드), onnxruntime 정적 라이브러리(약 360MB → `%LOCALAPPDATA%\ort.pyke.io`). 오프라인·CI 는 `LINDERA_BUILD_DICTIONARY_CACHE_DIR`(미검증)·`ORT_LIB_LOCATION`

## 저장소 구조

| 경로 | 내용 |
|---|---|
| `Cargo.toml` | Rust workspace — 공용 crate 는 `crates/` 에 추가. 빌드 산출물은 루트 `target/` |
| `src/` | React UI (Vite) — `components/ui/`(shadcn), `i18n/`(`locales/*.json`), `lib/ipc.ts`(invoke 래퍼), `test/setup.ts` |
| `crates/mam-db` | SQLite 계층 (연결·`DbPool`·설정·백업·마이그레이션 러너, `migrations/`) |
| `crates/mam-search` | Tantivy 풀텍스트, Outbox 색인 워커(`sync`), 검색 조정기(`coordinator`) |
| `crates/mam-vector` | 벡터 인덱스 (전수 비교 flat, 원시 임베딩 저장·세대 교체) |
| `crates/mam-core` | 자산 식별·파일 상태, 저장소(`storage`), 수집(스캔·감시·작업 대기열), 처리·프로토콜, 사용자 데이터·재활용, 보호(`protect`) |
| `crates/mam-media` | 형식 판별, FFmpeg/FFprobe 사이드카, 메타데이터, 썸네일, RAW 프리뷰, 장면전환 |
| `crates/mam-ai` | 모델 매니페스트·다운로드, ONNX 임베딩, whisper.cpp STT, 외부 AI·CLI 대리 실행 |
| `crates/mam-player`, `crates/mam-player-worker` | 플레이어 호스트 + libmpv 워커 프로세스 |
| `src-tauri/` | Tauri 앱 — `src/*_commands.rs`, `startup/`, `update_guard/`, `ai/`, `restore/`, `windows/hooks.nsh`, `tauri.conf.json`, `capabilities/` |
| `scripts/` | `dev-temp*.mjs`, `fetch-sidecars.ps1`, `build-player-worker.ps1`, `build-plugin-worker.ps1`, `dump-licenses.mjs`, `perf-e2e/` |

- 각 crate 의 `tests/` 는 통합 테스트(실제 SQLite·임시 디렉터리), `examples/` 는 측정 도구(`cargo test` 에서 실행 안 됨)

## 명령

```bash
pnpm install                 # 의존성 설치
pnpm tauri dev               # 개발 실행 (Vite :1420 + Rust 앱)
pnpm dev:temp                # 개발 실행 + 합성 테스트 라이브러리 (아래)
pnpm tauri build             # 설치 프로그램 빌드 (release) — updater 서명 키 필요 ("릴리스·업데이트")

pnpm typecheck               # TypeScript 검사
pnpm test                    # vitest 1회 실행 — 파일 지정: pnpm test src/lib/ipc.test.ts
pnpm test:watch              # vitest watch

cargo test --workspace       # Rust 테스트 (특정 crate·대상: cargo test -p <crate> --test <name>)
cargo fmt --all -- --check   # Rust 포맷 검사
cargo clippy --workspace --all-targets -- -D warnings
```

- **`tauri dev` 는 동시에 하나만** — 단일 인스턴스 뮤텍스 + Vite :1420. 병행 실행은 다른 식별자 빌드가 필요하다(`MAM_DATA_DIR` 만으로는 안 됨): `pnpm tauri build --no-bundle --config '{"identifier":"com.mam.desktop.<시험용>"}'`(`pnpm tauri dev` 도 같은 `--config`)
- 프로세스는 PID 로만 종료한다(이름 종료 금지 — `.claude/rules/project.md` §5)

### 개발용 temp 라이브러리 (`pnpm dev:temp`)

- `scripts/dev-temp-library.mjs` 가 ffmpeg 사이드카로 `D:\mam-desktop\library`(약 30개·5MB 합성 미디어)를 만들고(누락분만), `scripts/dev-temp.mjs` 가 `MAM_DATA_DIR=D:\mam-desktop\MAMData`, `MAM_DEV_SEED_FOLDER=D:\mam-desktop\library` 로 `tauri dev` 실행 — D: 가 없는 PC 는 `devRoot` 를 바꾼다. 디버그 전용 `src-tauri/src/dev_seed.rs` 가 관리 폴더가 없을 때만 등록
- CDP 자동 활성화(`--remote-debugging-port=19333` 을 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 에 추가, `CDP: http://127.0.0.1:19333/json/version` 출력). `MAM_DEV_CDP_PORT=<포트>` 재지정, `0` 이면 끔, 포트 사용 중이면 exit 1
- 초기화는 `D:\mam-desktop` 의 `MAMData`·`library` 삭제

## MAMData (데이터 디렉터리)

근거: [P2-1 작업 문서](docs/requests/2026-10/20261028-001256-p2-1-storage-foundation.md). 코드: `crates/mam-core/src/storage`, `crates/mam-db`, `src-tauri/src/{lib,storage_commands,error,logging,native}.rs`.

- **위치 결정 순서**: ① `MAM_DATA_DIR`(절대 경로, 시험·진단용, 검증 없음) ② `%LOCALAPPDATA%\MAMDesktop\location.json` (`{"dataDir": "D:\MAMData"}`) ③ 기본 `%LOCALAPPDATA%\MAMDesktop\MAMData`
  - `location.json` 이 손상·NAS·상대 경로면 기본 위치 + 경고. 위치 변경(`update_settings({ dataDir })`)은 다음 실행부터, 기존 데이터를 옮기지 않는다
  - 정상 기동마다 `%LOCALAPPDATA%\MAMDesktop\data-dir.txt`(UTF-16LE+BOM) 에 위치를 미러링 — `src-tauri/windows/hooks.nsh` 가 같은 순서로 읽는다
- **겹침 거부 (위반 시 원본 삭제 사고)**: 고른 폴더 아래 항상 `MAMData` 를 붙이고, MAMData·`backup.dir` 이 관리 폴더(원본)와 어느 방향이든 겹치면 `nested_folder`. 검사는 `storage::paths_overlap`
- **하위 구조**: `database/mam.sqlite`, `indexes/{tantivy,vectors/visual,vectors/semantic}`, `cache/{thumbnails,previews}`, `models/{embedding,stt}`, `backups/`, `logs/`, `updates/`, `tmp/`, `.lock`, `update-journal.json`, (있을 때만) `restore-pending.json`·`startup-notice.json`·`backup-location.json`·`database/mam.sqlite.corrupt-<ms>`. 경로는 `DataPaths` 한 곳에서만 만든다
- **기동 순서** (`lib.rs`): 위치 결정 → 세션 뮤텍스 → 디렉터리 → 로깅 → `.lock` → 위치 미러 → 업데이트 journal·reset·restore 적용 → 마이그레이션(적용 전 백업) → `DbPool`. 복구가 DB 열기보다 먼저여야 한다
- **단일 인스턴스**: 세션 뮤텍스 `Local\<identifier>.instance` + `.lock`. 두 번째 실행은 **창 제목으로 기존 창을 찾아** 전면화 후 종료 — 창 제목(`tauri.conf.json` `app.windows[0].title`)은 Rust `MAIN_WINDOW_TITLE` 과 정확히 같아야 한다
- **언어 폴더**: `%LOCALAPPDATA%\MAMDesktop\locales\<code>.json` (고정, MAMData 밖). 형식은 `ko.json` 과 같은 키 구조 + `"_meta": { "name": "<자국어 표기>" }`. 코드: `src-tauri/src/locale/`
  - **동봉 언어팩**: 저장소 `src-tauri/locales/{en,ja,es,fr,de}.json` → `tauri.conf.json` `bundle.resources`(`"locales/*.json": "locales/"`) → 설치 폴더 `locales\`(dev 는 `target/debug/locales`). 폴더에 `*.json` 이 하나도 없으면 빌드가 실패한다(glob 0건)
  - **없는 파일만 복사**: 설치(NSIS `hooks.nsh` `NSIS_HOOK_POSTINSTALL`, 자동 업데이트 포함)와 앱 시작(`locale::seed`, dev·삭제 복구용) 두 곳에서 언어 폴더에 같은 이름이 없을 때만 복사 — 사용자 수정본은 덮어쓰지 않으므로 **업데이트로 새 문구가 와도 기존 사본은 갱신되지 않는다**(파일을 지우면 다음 실행에 최신본 복사)
  - **내장 언어 `en`(기본·폴백) + `ko`**: 프론트 번들에 둘 다 들어간다(`en` 은 `src-tauri/locales/en.json` 을 직접 import). 설정 `language` 기본값은 `en`(Rust `DEFAULT_LANGUAGE`). 폴더의 `en.json`/`ko.json` 은 내장 위 키 단위 덮어쓰기. 마지막 내장 언어를 localStorage `mam.uiLanguage` 에 기억해 시작 깜빡임을 막는다. vitest 는 `src/test/setup.ts` 에서 `ko` 로 시작한다
  - **`ko.json` 에 키를 추가하면 5개 언어팩에도 모두 추가**한다 — vitest `T-ui-i18n-03~05` 가 누락·자리표시자 불일치·빈 값을 실패로 잡는다
- **DbPool**: 쓰기 1개(`High`/`Low` 큐) + 읽기 4개. `pool.write`/`pool.read` 는 블로킹 → async command 에서는 `spawn_blocking`. 자산 변경과 `search_outbox` 는 같은 쓰기 클로저(한 트랜잭션)
- **설정**: `crates/mam-db/src/settings.rs` `DEFS`. 새 설정은 `DEFS` + `Settings` 구조체 + `src/lib/ipc.ts` `Settings` 를 함께 고친다
- **백업**: `backup_now` → `backups/mam-<unix ms>.sqlite`, **SQLite 백업 API 로만**(실행 중 파일 복사 금지). 오류는 `{ code, message }`(`ErrorCode`)로 reject, message 에 내부 경로 금지
- **로그**: `logs/mam.<날짜>.log` + `logs/startup.log`. `password`·`api_key`·`token`·`secret` 이름 필드는 `[redacted]` — **비밀은 구조화 필드로만, 메시지 문자열에 넣지 않는다**

## DB 마이그레이션 (`crates/mam-db`)

rusqlite (bundled, backup) + refinery. 적용 이력은 `refinery_schema_history`. 규칙 정본은 `.claude/rules/db.md` §5.

- **추가**: `crates/mam-db/migrations/V<다음 번호>__<snake_case 이름>.sql`. 연속 정수, 빌드 때 포함(`embed_migrations!`). 브랜치 충돌 시 나중에 머지하는 쪽이 번호만 바꾼다
- **추가형만**(테이블·인덱스·NULL/기본값 컬럼). **적용된 마이그레이션은 수정하지 않는다** — 체크섬 오류로 기동이 멈춘다. 업데이트 롤백 시 구 앱이 새 스키마를 읽어야 한다
- **예외: `V1__core_schema.sql` 은 DRAFT** — 첫 외부 빌드 전까지 수정 가능. 수정하면 개발 DB 가 체크섬 오류를 내므로 개발 DB 파일을 지우고 다시 만든다. 동결 이후에는 예외 없음
- 대기 마이그레이션은 한 트랜잭션으로 적용. SQL 안에 `BEGIN`/`COMMIT`·**`PRAGMA` 금지**(무효 — FK OFF/`foreign_key_check` 는 러너가 처리). 테이블 재생성은 12단계 방식(새 테이블 → 복사 → `DROP TABLE` → `RENAME` → 인덱스·트리거)
- 적용 전 백업 `mam-pre-migrate-v<현재 버전>-<unix ms>.sqlite`. DB 가 앱보다 새로우면 `SchemaTooNew`, 같은 버전 둘이면 `DuplicateMigration`
- 쿼리·필터 추가 시 `crates/mam-db/tests/query_plans.rs` 방식으로 `EXPLAIN QUERY PLAN` 확인

```bash
cargo test -p mam-db                      # DB 계층 전체 (단위 + 통합)
cargo test -p mam-db --test migrate_apply --test migrate_guards   # 마이그레이션 러너만
cargo test -p mam-db t_db_09              # 테스트 ID 로 선택
```

## 기능별 개발 메모

### 풀텍스트·벡터 검색 (`crates/mam-search`, `crates/mam-vector`, `mam-core::query`)

- tantivy + lindera(`embed-ko-dic`, 사전 Apache-2.0). **분석기·문서 필드를 바꾸면 `ANALYZER_VERSION` 을 올린다**(`mam-search.meta` 불일치 → SQLite 에서 재구축)
- Outbox 워커 `mam_search::sync::SyncWorker`(SQLite 상태 기준, 최신 revision 만, 백오프는 호출자). 조정기 command `src-tauri/src/search_commands.rs`, 워커·필터 `search_service.rs`
- 시험: `cargo test -p mam-search --test coordinator --test coordinator_sync`, `-p mam-core --test query_service`, `-p mam-desktop --lib search_service`(T-ipc-30~35), `pnpm test`(T-ui-ipc-40~42) — 측정: [measurements.md — 풀텍스트](docs/dev/measurements.md#풀텍스트-검색-p1-2-mam-search), [검색 동기화·조정기](docs/dev/measurements.md#검색-동기화조정기-p2-4p2-5), [종단 성능](docs/dev/measurements.md#종단-성능-113-p4--합성-라이브러리를-실제-앱에-적재)
- 벡터(`crates/mam-vector`, `mam_core::embedding`): 엔진은 전수 비교 flat 하나. `mam-vector.meta` 불일치 → 새 세대 구축 후 `IndexRoot::activate`, **view(mmap) 핸들 해제 뒤** `retire`. `put` 을 거치지 않고 `embeddings` 를 바꾸는 새 경로는 `SearchService` 에서 `self.vectors.cache.invalidate_all()` 을 호출
- 시험: `cargo test -p mam-vector --test embed_store`, `cargo test -p mam-core --test embedding` — 측정: [measurements.md — 벡터](docs/dev/measurements.md#벡터-p1-3-mam-vector)

### 자산 식별·수집 (`crates/mam-core`)

- 스캔: `register_volume` → `ScanSession::begin` → `Reconciler::reconcile` → **끝까지 완료됐을 때만** `Reconciler::finish_scan`(중간 실패에 부르면 멀쩡한 파일이 `missing`)
- 해시는 쓰기 트랜잭션 밖. 원본은 읽기 전용으로만 연다. 폴더 하위 조회는 `path_key` 범위(`mam_core::key_range`) — `LIKE` 금지
- 시험: `cargo test -p mam-core` (통합은 `#![cfg(windows)]`, `tests/jobs.rs`·`ingest_scan.rs`·`ingest_watch.rs`, 읽기 거부 시험은 `icacls`) — 측정: [해시 I/O](docs/dev/measurements.md#해시-io-p1-4-mam-core), [스캔·처리](docs/dev/measurements.md#스캔자산-처리-p2-2p2-3-mam-core)

### 자산 처리·미디어 (`mam-core::{processing, protocol}`, `crates/mam-media`)

- `Ingest::enable_processing` 을 `start()` 전에 호출해야 메타·썸네일·해시 작업이 생긴다. 산출물은 `MAMData/cache` 아래에만(원본 폴더 쓰기 금지)
- FFmpeg 경로: 설정 → exe 옆 사이드카 → `PATH`. 개발 PC 의 GPL FFmpeg 는 **시험 전용**
- 프로토콜: `http://mamthumb.localhost/<asset_id>/<320|1024|strip>`, `http://mammedia.localhost/<asset_id>` (어댑터 `src-tauri/src/protocols.rs`, 프론트 `src/lib/media-urls.ts`)
- 장면전환(`mam_media::scene`): 명령·임계값을 바꾸면 `SCENE_DETECTOR_VERSION` 을 올린다 (설계 `docs/requests/2026-11/20261113-212724-architect-scene-cuts-design.md`)
- 시험: `cargo test -p mam-core --test processing --test processing_protocol --test processing_merge`, `-p mam-media --test metadata_strip_wic`, `-p mam-db --test processing_schema` (ffmpeg·HEIF 코덱 없으면 SKIP) — 측정: [샘플·호환표](docs/dev/measurements.md#샘플-생성호환표-p1-5-mam-media)

### 로컬·외부 AI (`crates/mam-ai`, `src-tauri/src/ai/`)

- 네트워크는 모델 최초 다운로드에서만. 모델 `MAMData/models/<model_id>/`, 매니페스트 `src/models/manifest.json`(URL·SHA-256 고정 — 갱신 시 해시 재계산). **저장 벡터는 fp32 모델로만**(int8/uint8 은 출력이 달라짐)
- 동의(`give_ai_consent`) 전 외부 호출 0회, 원본 음성 외부 전송 금지. API 키는 `set_api_key` 인자로만 들어오고 응답·로그에 나가지 않는다(DPAPI 저장)
- 정책 PA1~PA9: `docs/requests/2026-10/20261029-005836-p3-policies-and-wiring.md`. 이벤트 계약 `src/lib/ipc-contract-ai.ts`
- 시험: `cargo test -p mam-ai`, `cargo test -p mam-desktop --lib ai::` (네트워크·모델 불필요). 실제 모델 시험은 `MAM_AI_TEST_MODELS`, e2e STT 는 `MAM_AI_TEST_WHISPER`·`MAM_AI_TEST_STT_WAV` 가 있을 때만 — 측정: [로컬 AI](docs/dev/measurements.md#로컬-ai-p1-6-mam-ai)

#### Claude Code CLI 대리 실행 (`mam-ai::external::cli`)

- 근거: `docs/requests/2026-11/20261107-135215-ai-subscription-auth.md`. `claude --version` ≥ `MIN_VERSION`(2.1.280)
- **격리 인자는 `args.rs` 에 고정** — `--tools ""`·`--setting-sources` 만으로는 격리되지 않는다(`.claude/rules/project.md` §5). 인증 환경변수(`ANTHROPIC_API_KEY`/`ANTHROPIC_AUTH_TOKEN`)는 지우지 말고 감지만
- 시험: `cargo test -p mam-ai --lib cli`, `cargo test -p mam-desktop --lib ai::cli_tests` (실제 `claude` 미실행)
- 수동: `pnpm tauri dev` → 설정 > AI > Claude > "구독 계정" → [다시 확인]·[연결 확인]·자산 1건 분석. 오프라인은 `HTTPS_PROXY` 를 죽은 주소로

### 사용자 데이터·재활용 (`userdata`, `reuse`)

- 변경은 한 트랜잭션에서 데이터 + 실제로 바뀐 자산의 `revision` + `search_outbox`
- OS 부수효과(`ExportFs`·`Launcher`·`Clipboard`)는 trait 뒤 — 시험은 실제 창·클립보드를 건드리지 않는다
- 시험: `cargo test -p mam-core --test user_data --test reuse`, `-p mam-db --test user_data_schema`, `-p mam-search`(T-sync-13~15)

#### 사용자 정의 메타데이터 필드 (`userdata::custom_fields`, V19)

- 설계: `docs/requests/2026-11/20261115-135644-custom-metadata-fields.md`. 원천은 SQLite 3 테이블(`metadata_fields`·`metadata_field_options`·`asset_metadata_values`), Tantivy 는 `umeta`(JSON, raw, INDEXED|FAST)·`umeta_has`·`umeta_text` 파생 필드 — V19 와 함께 `ANALYZER_VERSION` 9 로 올라 **첫 실행 시 기존 색인 전체 재구축**(업데이트 롤백 후 구 앱이 열 때도 재구축)
- Tantivy 키는 `f{field_id}` — 필드 이름·선택지 이름·순서 변경은 재색인 없음. `number` 일치는 term 이 아니라 `[x, x]` 범위로 질의(정수처럼 보이는 f64 는 i64 term 으로 색인됨)
- 필드·선택지 삭제는 즉시 `deleting_at` 표시(이름 재사용 가능) 후 `purge_pending_metadata_deletions()` 가 5,000 자산 단위 트랜잭션으로 정리 — 삭제 command 직후와 앱 시작 시(`mam-umeta-purge` 스레드) 실행. 새 읽기 경로는 `deleting_at` 이 있는 필드·선택지를 제외해야 한다
- 검색 조건의 필드 존재·타입 검증은 `src-tauri/src/search_user_fields.rs`(서비스 계층) — `mam-search` 는 구조 검증만
- 시험: `cargo test -p mam-core --test user_metadata_defs --test user_metadata_values --test user_metadata_purge`, `-p mam-db --test custom_metadata_migration`, `-p mam-search --test user_fields_filter --test user_fields_sync`, `-p mam-desktop --lib metadata_commands`(T-ipc-umeta-*), `pnpm test`(T-ui-umeta-*). PoC: `crates/mam-search/examples/umeta_poc/` (100만 건 합성, 실행 후 cargo target 디렉터리의 `bench-umeta` 약 0.8GB 삭제)

### 보호·운영 (`mam-core::protect`, `src-tauri/src/{protection,protect_commands,reset,restore,backup_scheduler,tray}`)

- 비밀번호 해시는 `mam_db::settings::{load_password_hash, swap_password_hash}` 로만. 초기화는 `database/`·`indexes/`·`cache/` 만 지운다(원본·백업·모델·로그 제외)
- dev 에서 `app.restart()`(초기화·복원)는 CLI·Vite 까지 끝낸다 — 재시작된 PID 를 종료하고 `tauri dev` 를 다시 띄운다
- 스모크: 관리 폴더에는 픽스처 사본만. 실제 휴지통 시험은 `MAM_TEST_REAL_RECYCLE_BIN=1` 일 때만
- 시험: `cargo test -p mam-core --test protect_auth --test protect_originals --test protect_move --test protect_reset`, `cargo test -p mam-desktop --lib`(T-prot-12·16~25), `-p mam-db --test settings_backup`(T-prot-13)

### 플레이어 (`crates/mam-player` + `crates/mam-player-worker`, libmpv 워커)

- 구조: 앱(`PlayerHost`) ↔ Named Pipe(JSON 줄) ↔ `mam-player-worker.exe`(libmpv 동적 로드) — 프레임은 WebView2 SharedBuffer 3개 + WebGL2 canvas. 계약 정본: `docs/requests/2026-11/20261119-192800-architect-libmpv-ipc-contract.md`
- 탐색 순서: 워커 `MAM_PLAYER_WORKER` → `<exe 폴더>`(테스트는 한 단계 위 `target/debug`) / libmpv `MAM_LIBMPV_DIR` → `<exe 폴더>/libmpv/` → (디버그만) `src-tauri/binaries/libmpv`, `temp/poc-mpv/lib`. 워커는 workspace 멤버라 `cargo build` 가 만든다
- 시험: `cargo test -p mam-player`(프로토콜·감독자·가짜 워커), `cargo test -p mam-player-worker`(실제 워커 + libmpv live 시험 — libmpv·시료(`MAM_PLAYER_SAMPLE_DIR`, 기본 `temp/poc-mpv/samples`)가 없으면 SKIP 출력 후 통과). live 시험은 소리를 끄고(`MAM_PLAYER_AO=null`) 시료를 임시 폴더로 복사해서만 쓴다
- 워커 개발용 환경변수: `MAM_PLAYER_AO`(예 `null`), `MAM_PLAYER_RENDER_API`(`sw`|`d3d11`), `MAM_PLAYER_LOG`(mpv 경고를 stderr 로)
- **wry/tauri 업그레이드 체크리스트**: 객체 메시지(`chrome.webview.postMessage(object)`)를 wry 의 `add_WebMessageReceived` 핸들러(`TryGetWebMessageAsString` 실패 → 무시)가 계속 무시하는지, `attach_ipc_handler` 의 비문자열 처리가 바뀌지 않았는지 확인 (`wry-0.57.1/src/webview2/mod.rs`). 바뀌면 플레이어 슬롯 반환이 Tauri IPC 로 새거나 로그가 폭주한다

### 플러그인 (`crates/mam-plugin-{manifest,host,worker,gateway,runtime}`)

설계·결정 정본: `docs/requests/2026-11/20261122-170245-plugin-runtime-implementation-plan.md` 와 같은 접두어의 `architect-plugin-*-spec`. 샘플: `examples/plugins/exif-summary/`, 작성자용 타입: `examples/plugins/plugin-api.d.ts`(`/// <reference path>` 로 참조 — `tsc --ignoreConfig --noEmit --allowJs --checkJs main.js` 로 점검). 플러그인은 **JS(QuickJS-NG) 한 파일**이며 별도 워커 프로세스에서 돈다 — 모듈·Node API·네트워크·파일·타이머가 없고 전역 `mam` 만 있다. 원본 미디어를 쓰는 API 는 없다.

- **폴더 구성**: `plugin.json`(필수) + `main` 이 가리키는 `.js` 1개 + (선택) `l10n/<코드>.json`. 설치는 이 3종류만 복사한다(`main.js` ≤ 2 MiB, 로케일 파일 ≤ 64개)
- **manifest** (`apiVersion: 1`, 모르는 필드는 거부): `id`(역도메인 `com.example.x`, 폴더명이 됨) · `version`(엄격 semver) · `engines.mam`(`>=` 권장) · `main`(상대 `.js`, `..`·`\`·`:` 불가) · `displayName`·`description`(**번역 키** — 문자 그대로 표시 안 됨) · `permissions` · `activationEvents`(선택, `onCommand:<id>`/`onProvider:<id>`) · `contributes.commands[]{id,title}` · `contributes.menus.assetContext[]{command,when?}` · `contributes.providers.{metadata,analysis}[]{id,title,assetKinds?}` · `providers.export[]{…,fileExtension}`(`export.write` 필수, 확장자는 데이터 형식 허용 목록만: csv·tsv·json·jsonl·txt·md·xml·xmp·srt·vtt·yaml·yml — 실행·스크립트 형식(exe·bat·js·html·svg 등)은 설치가 거부되고 내보내기 때도 다시 확인). 문자열 표시 순서: 현재 로케일 → `en` → 키 이름
- **권한 6종**(선언 ∩ 사용자 허용만 유효, 호출마다 검사): `asset.read.metadata`, `asset.read.derived`(썸네일), `kv.read`, `kv.write`, `tag.write`, `export.write`. 필요한 것만 선언한다. 네트워크·프로세스 실행·원본 쓰기 권한은 없다
- **`when`**(메뉴 항목 표시 조건): 식별자는 `selection.count`(수) · `asset.kind`(`'image'|'video'|'audio'`) · `asset.offline`(불리언) 뿐. 연산자 `! && || == != < <= > >=`(비교는 비결합, `<`류는 숫자만), 괄호, 문자열 `'..'`/`".."`, 수·`true`/`false`. ≤ 256 바이트·깊이 ≤ 16. 모르는 식별자·타입 오류는 **설치 실패**, 실행 시 오류는 거짓(숨김). 다중 선택의 `asset.*` 는 우클릭한 자산. 시험 벡터 `crates/mam-plugin-manifest/vectors/when.json`(Rust·TS 공용)
- **진입점**: `mam.commands.register(id, ({assetIds}) => string|void)` — 문자열은 토스트(제어문자 제거, 200자). `mam.providers.register(id, ({assetId, kind, revision}) => …)` — metadata/analysis 는 자산 변경 때 자동 호출, 반환 `{kv?: {키: JSON}, tags?: string[]}`(kv ≤ 32개, tags 1~20개)를 호스트가 한 트랜잭션으로 기록. export 는 `({assetIds, kind:'export'})` → `{text}` 또는 `{dataBase64}`(자산 ≤ 5,000개, 디코딩 후 ≤ 3 MiB − 4 KiB, 사용자가 고른 경로에 호스트가 저장). 핸들러는 **동기**여야 한다
- **호스트 API**(`mam.host.call(method, params)`, 오류는 `e.code`): `asset.get` · `asset.getDerived`(썸네일) · `kv.get/set/delete/list` · `tag.add`(제거 API 없음). 로그는 `mam.log.{debug,info,warn,error}`. 호출은 **선택·작업 대상 자산**만 `assetId` 로 쓸 수 있다(밖은 `not_found`). `l10n.t` 는 1차 범위에서 제외 — 명령 결과 메시지는 플러그인이 돌려준 문장을 **그대로** 표시한다(앱이 번역하지 않음, 제어·서식 문자만 제거). 오류 코드: `permission_denied bad_request unknown_method not_found too_large quota_exceeded unavailable busy internal`
- **한도**(제안값, 미측정): QuickJS 힙 64 MiB · 스택 512 KiB · 호출당 CPU 2 s · 호출 1회 10 s · **호출당 메시지(host.call·log) 10,000** · KV 값 64 KiB · KV 총량 플러그인당 256 MiB · 자산당 태그 100개 · 호출당 새 태그 200개 · 썸네일 1 MiB(호출당 32 MiB). 결함(크래시·타임아웃·한도 위반) 10분 안에 3회면 자동 비활성. 명령이 자산 수에 비례해 호출하면 메시지 예산을 넘지 않게 상한을 둔다(샘플은 500)
- **개발 중 설치**: 앱 설정 > 플러그인 > "폴더에서 설치…" 에서 플러그인 폴더(절대 경로)를 고른다 → 미리보기(권한·서명 없음 표시) → 동의 → 설치. command 는 `plugin_inspect_dir`·`plugin_install`. 소스 폴더는 읽기만 하고 설치본은 복사본이라 **소스를 고친 뒤 다시 설치**(같은 버전이면 재설치, 올리면 업그레이드). MAMData 와 겹치는 폴더는 거부
- **데이터 위치**: 패키지 `MAMData/plugins/<id>/<version>/`(`.staging`·`.trash` 는 설치 중 임시), 상태는 `settings` 의 `plugin_installed|enabled|granted|feed_cursor:<id>`, 데이터는 SQLite `plugins`·`plugin_providers`(정수 키 레지스트리, `plugin_ref`/`provider_ref`)·`plugin_kv`(플러그인별 네임스페이스)·`plugin_tasks`(provider 작업). **이전 V26/V27(문자열 키 스키마)로 만든 개발용 DB·백업은 삭제해야 한다** — V26 내용이 바뀌어 마이그레이션 체크섬이 맞지 않는다
- **디버그**: 워커 실행 파일은 `MAM_PLUGIN_WORKER_EXE`(파일, 개발용) → `<exe 폴더>`(시험은 `deps` 의 한 단계 위 `target/debug`)에서 찾는다. 워커는 workspace 멤버라 `cargo build`(또는 `-p mam-plugin-worker`)가 만들지만 `tauri dev` 는 앱만 빌드하므로 한 번 먼저 빌드한다 — 없으면 `workerAvailable=false` 로 호출이 실패한다. 플러그인 로그(`mam.log`)는 `logs/mam.<날짜>.log` 의 target `mam::plugin`(필드 `plugin_id`), warn 이하만 — 제어문자 제거·비밀 형태 문자열 마스킹·초당 20건 제한, `debug` 는 기본 꺼짐. 호스트 오류는 플러그인에 `internal` 로만 보이고 상세는 앱 로그에 있다. 자동 비활성된 플러그인은 설정 화면에서 다시 켠다
- **제거·복원**: 제거 시 "데이터 유지(`keep_data`)" 면 KV·작업 행을 남겨 재설치하면 다시 보이고, 아니면 5,000행 배치로 지운다(원본 미디어는 건드리지 않음). **백업 복원 후에는 현재 설치되지 않은 플러그인의 설정·KV·작업이 자동 삭제**되고, 활성·허용 상태는 현재 ∩ 백업이며 `MAMData/plugins` 패키지는 되감기지 않는다(`src-tauri/src/restore/plugins.rs`)
- **시험**: `cargo test -p mam-plugin-manifest -p mam-plugin-host -p mam-plugin-gateway -p mam-plugin-runtime`. 통합 시험은 실제 워커·SQLite 를 쓰며 시험 프로세스가 `cargo build -p mam-plugin-worker` 를 먼저 실행한다. 샘플 시험 `T-psample-*` 는 `crates/mam-plugin-runtime/tests/sample_exif.rs`

## 릴리스·업데이트 (NSIS + Tauri updater)

근거·시나리오: [P1-7 작업 문서](docs/requests/2026-10/20261028-001121-p1-7-update-rollback.md). 로컬 시험 절차: [measurements.md](docs/dev/measurements.md#로컬-업데이트-시나리오-시험-p1-7).

- **설치기**: NSIS(`bundle.targets` = `nsis`), 현재 사용자 설치 `%LOCALAPPDATA%\velio`. 제거해도 MAMData 는 남는다. Authenticode 코드 서명 없음(SmartScreen 경고)
- **updater 서명**: minisign. **개인 키는 저장소에 절대 넣지 않는다**. `plugins.updater.pubkey`·엔드포인트(`updates.example.invalid`)는 시험용 자리표시자 — 릴리스 전 교체
  ```bash
  pnpm tauri signer generate -w <안전한 경로>/mam-updater.key     # 키 쌍 생성 (.key 비공개, .key.pub 공개)
  TAURI_SIGNING_PRIVATE_KEY=<키 파일 경로 또는 내용> TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<암호> \
    pnpm tauri build --config release.conf.json --config src-tauri/tauri.sidecars.conf.json
  # release.conf.json: {"plugins":{"updater":{"pubkey":"...","endpoints":["https://..."]}}}
  ```
  - `createUpdaterArtifacts` 가 켜져 있어 키가 없으면 서명 단계에서 실패. `requireSignedVersion` 으로 다운그레이드 방지. `--config` 는 여러 번 가능(뒤가 우선)
- **`latest.json`** (정적 파일, `.github/scripts/make-latest-json.mjs`): `{"version","notes","pub_date","platforms":{"windows-x86_64":{"signature":"<.sig 파일 내용>","url":"<설치기 URL>"}}}`. https 만 허용(시험용 http 는 `dangerousInsecureTransportProtocol`)
- **업데이트 흐름**: `check_update` → 사용자 승인 → `install_update`(자동 설치 없음) → DB 스냅샷 `backups/mam-pre-update-v<버전>-<ms>.sqlite` + `update-journal.json`(`pending`) → 훅이 새 exe 로 `--verify-update`(DB 열기·마이그레이션, UI 없음, 종료 코드)
  - 실패하면 이전 설치기 `MAMData/updates/current-setup.exe` 를 조용히 재실행하고 구 버전이 첫 기동에서 DB 를 복원. 새 버전이 `healthy` 에 못 간 채 3번 실패해도 4번째 기동에서 같은 복구
  - 이전 설치기가 없으면 시작하지 않는다(`previous_installer_missing`) — 훅이 설치마다 `MAMData/updates/` 에 사본을 남기고 정상 기동 시 승격
- **journal 계약** (`src-tauri/src/update_guard/journal.rs`): `from_version`·`to_version`·`previous_installer`·`db_backup`·`state`(`pending`→`healthy` / `rolling_back`→`rolled_back`)·`failed_starts`·`created_at_ms`·`updated_at_ms`. 파싱 실패 파일은 보존 후 무시. 바꾸면 이 절도 갱신
- **`hooks.nsh` 계약**: MAMData 위치를 앱과 같은 순서(`MAM_DATA_DIR` > `data-dir.txt` 미러 > 기본)로 읽고, 설치 후 `--verify-update` 검증·실패 시 이전 설치기 실행, 설치기 사본 보관, 바로가기·`DisplayName` 의 `velio.` 교체
- **실패 주입 기능 플래그** (제품 빌드 금지): `--features inject-fail-verify`, `inject-fail-startup`. `cargo test -p mam-desktop` 은 플래그 없이
- **시험 설치 주의**: 설치기는 실행 중인 `velio.exe` 를 종료시킨다. 설치된 앱과 개발 앱은 데이터 경로(`%LOCALAPPDATA%\MAMDesktop`)를 공유하므로 동시 실행 금지 — 시험 설치는 `--config` 로 `identifier`·`productName` 을 바꿔서

### 브랜드 (`velio.`)

- 표시 이름은 `velio.` 이지만 **`productName` 은 `velio`** — Windows 폴더명은 마침표로 끝날 수 없으므로 끝 마침표 금지. 바로가기(`velio..lnk`)·`DisplayName` 은 `hooks.nsh` 가 교체
- `mainBinaryName` `velio` → 설치 실행 파일 `velio.exe`(`hooks.nsh` `.onGUIEnd` 만 `velio.exe` 리터럴). 개발 빌드는 `target\debug\mam-desktop.exe` 그대로
- **의도적으로 유지(바꾸면 기존 설치 데이터·단일 인스턴스가 깨짐)**: `identifier` `com.mam.desktop`, Cargo 패키지명 `mam-desktop`, 데이터 경로 `%LOCALAPPDATA%\MAMDesktop`
- 아이콘: 원본 `src-tauri/app-icon.svg` → `pnpm tauri icon src-tauri/app-icon.svg` 로 `src-tauri/icons/*` 재생성 후 쓰지 않는 `android/`·`ios/`·`64x64.png` 삭제

## 사이드카 · 서드파티 고지

설치기는 FFmpeg/FFprobe, whisper.cpp, libmpv(공식 LGPL 빌드, 플레이어 워커가 동적 로드)와 플레이어 워커 `mam-player-worker.exe` 를 동봉한다. **바이너리는 커밋하지 않는다**(`src-tauri/binaries/` gitignore).

```powershell
./scripts/fetch-sidecars.ps1          # 받기 + SHA-256 검증 + 배치 (이미 있으면 건너뜀, -Force 재설치, -CacheDir zip 캐시 위치)
pnpm tauri build --config src-tauri/tauri.sidecars.conf.json   # 사이드카를 번들에 포함 (updater 서명 키 필요). beforeBuildCommand 가 프런트 빌드 뒤 scripts/build-player-worker.ps1 로 워커를 릴리스 빌드한다
```

- 구성: FFmpeg n8.1.3 `win64-lgpl-shared`(BtbN 고정 태그), whisper.cpp `b5454` `whisper-bin-x64.zip`, libmpv mpv v0.42.0 공식 릴리스 자산 `libmpv-v0.42.0-x86_64-w64-mingw32-lgpl.zip`(그중 `libmpv-2.dll`·`LICENSE.LGPL` 만 꺼냄). URL·해시는 스크립트 상단 상수 — 올릴 때 SHA-256 을 직접 계산해 함께 바꾸고 BtbN `latest` 태그는 쓰지 않는다
- **LGPL 제약**: FFmpeg 는 `--enable-gpl`·`--enable-nonfree` 없는 빌드만(스크립트가 검사 후 중단). libmpv 는 DLL 에 박힌 FFmpeg 설정이 `--enable-gpl`·`--enable-nonfree`·`license: GPL`·FFTW 이면 스크립트가 중단하고(실패 시 `binaries/libmpv` 삭제), LGPL 표지가 없어도 중단 — libmpv 를 올릴 때 URL·SHA-256 을 직접 계산해 함께 바꾼다. 공식 v0.42.0 빌드는 libzvbi(GPL-2.0-only 파일 포함)를 링크해 경고가 나온다 — 릴리스 전 해결 사항(`THIRD_PARTY_NOTICES.md` 1.3.1)
- 배치: `binaries/{ffmpeg,ffprobe,whisper-cli}-x86_64-pc-windows-msvc.exe`(`externalBin`), `binaries/dlls/*`, `binaries/licenses/*`, `binaries/libmpv/`(`libmpv-2.dll`+`LICENSE.LGPL`, 디렉터리 경로 매핑 → `<설치 폴더>/libmpv/` — `…/*` 글롭은 평탄화됨), `binaries/mam-player-worker-x86_64-pc-windows-msvc.exe`(`externalBin` — Tauri 가 접미사를 떼고 `velio.exe` 옆에 `mam-player-worker.exe` 로 설치, 호스트는 exe 폴더에서 찾는다). 사이드카 설정은 **오버레이로만** — `externalBin` 파일이 없으면 모든 cargo 빌드가 실패하므로 기본 `tauri.conf.json` 에 넣지 않는다(빠진 설치기에는 FFmpeg/whisper/libmpv/워커가 없어 내장 플레이어가 `player_available=false`)
- **개발에서 플레이어 쓰기** (`tauri dev`): ① libmpv — `./scripts/fetch-sidecars.ps1` 를 한 번 실행하면 디버그 빌드가 `src-tauri/binaries/libmpv` 를 찾는다. 다른 폴더를 쓰려면 `MAM_LIBMPV_DIR`(= `libmpv-2.dll` 이 있는 폴더, 릴리스 빌드도 따름). ② 워커 — `cargo build -p mam-player-worker`(또는 `cargo build --workspace`) 를 먼저 한 번: `cargo run`/`tauri dev` 는 앱 패키지만 빌드해 `target/debug/mam-player-worker.exe` 가 없을 수 있다(`MAM_PLAYER_WORKER` 로 파일 지정 가능). 둘 중 하나라도 없으면 앱은 뜨고 플레이어만 `player_available=false` 로 저하된다. 설치기용 워커는 `tauri build` 가 알아서 만든다(`scripts/build-player-worker.ps1`, `--locked` 릴리스 빌드 → `src-tauri/binaries/` 로 복사)
- **플러그인 워커**: `mam-plugin-worker.exe`(QuickJS-NG v0.16.2 — rquickjs 0.14 이 동봉, Bellard QuickJS 아님, 약 1.3 MB)는 플레이어 워커처럼 `velio.exe` 옆에 설치된다. `scripts/build-plugin-worker.ps1` 이 `--locked` 릴리스로 빌드해 `src-tauri/binaries/mam-plugin-worker-x86_64-pc-windows-msvc.exe` 로 두고, `tauri.sidecars.conf.json` `beforeBuildCommand` 에 플레이어 워커 스크립트 뒤로 이어져 있다. 탐색 순서는 `MAM_PLUGIN_WORKER_EXE`(파일, 개발용) → exe 폴더(시험은 `deps` 의 한 단계 위 `target/debug`). 개발에서는 workspace 멤버라 `cargo build` 가 만든다
- **고지**: 루트 `THIRD_PARTY_NOTICES.md`(수기)와 `THIRD_PARTY_LICENSES.md`(자동 생성, 영문)가 설치 폴더에 설치된다(`tauri.conf.json` `bundle.resources`). **동봉 바이너리·모델을 바꾸면 NOTICES 를 갱신**한다(FFmpeg 버전·커밋·태그 포함)
- **`THIRD_PARTY_LICENSES.md` 생성 범위**: 배포되는 것만 — Rust 는 `normal` 의존성 간선만(build·dev·proc-macro 제외), npm 은 Vite 프로덕션 빌드(메모리 실행)에 렌더된 모듈 + 앱 CSS `@import` 패키지, 동봉 프로그램은 `THIRD_PARTY_NOTICES.md` 1.1.1·1.2·1.3.1 표의 모든 라이선스 원문 + onnxruntime 고지. 패키지별 저작권·전문·NOTICE 를 전문별로 묶는다. 결과는 커밋하며 **CI·빌드는 생성하지 않는다** — 의존성을 바꾼 뒤 갱신이 필요하다고 판단하면 **별도 작업 요청으로** 재생성·커밋
  ```bash
  node scripts/dump-licenses.mjs           # 재생성 (허용 목록 밖·UNKNOWN·저작권 미확인이면 종료 코드 1, 파일은 쓰지 않음)
  node scripts/dump-licenses.mjs --check   # 쓰지 않고 Cargo.lock·pnpm-lock.yaml SHA-256 과 패키지 증감만 출력 (종료 코드 0)
  ```
  - 허용 라이선스·예외(사유 필수)·저작권 확인 목록: `scripts/licenses/policy.mjs`. 고정 원문(LGPL·GPL·MPL·OFL·CDLA·onnxruntime 고지·SPDX 대체 전문)과 출처·SHA-256: `licenses/` (`README.md`). `ort` 버전을 올리면 해당 태그의 `ThirdPartyNotices.txt` 로 교체
  - `licenses/GPL-3.0.txt` 는 `fetch-sidecars.ps1` 이 설치 폴더 `licenses\` 에 `ffmpeg-LICENSE.txt` 와 나란히 복사한다(LGPL-3.0 이 GPL-3.0 을 참조)
- **릴리스 가드**: 고지 파일에 자리표시자 연락처(`opensource@velio.example.invalid`)가 남아 있으면 `release.yml` 이 실패한다. 실제 연락처로 교체한 뒤 릴리스
- 릴리스 전 점검: `node scripts/dump-licenses.mjs --check` (강제 아님 — 갱신 여부는 위 규칙대로 판단)

## CI (GitHub Actions)

- `.github/workflows/ci.yml` (push / PR / 수동): `frontend`(`install --frozen-lockfile` → `typecheck` → `test`), `rust`(`cargo fmt --all --check` → clippy → `cargo test --workspace`, `--all-features` 없음), `build`(서명 없는 NSIS 설치기). cargo 잡은 gitignore 된 `dist/` 대신 빈 `dist/index.html` 자리표시자를 만든다
- CI 설치기: `./scripts/fetch-sidecars.ps1` 후 `pnpm tauri build --config .github/tauri.ci.conf.json --config src-tauri/tauri.sidecars.conf.json`
- 캐시: `LINDERA_BUILD_DICTIONARY_CACHE_DIR`(= `$RUNNER_TEMP/lindera-dict-cache`), `%LOCALAPPDATA%\ort.pyke.io`, `Swatinem/rust-cache`, pnpm store
- `.github/workflows/release.yml` (태그 `v*` / 수동): 서명 입력이 없거나 태그≠`tauri.conf.json` 버전이면 실패. 설치기 + `.sig` + `latest.json` 스텁, 태그면 초안 릴리스
- 시크릿/변수: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, (선택) `WINDOWS_CERT_PFX_BASE64`, `WINDOWS_CERT_PASSWORD`; variable `UPDATER_PUBKEY`, `UPDATER_ENDPOINT`, (선택) `UPDATER_DOWNLOAD_BASE_URL`

## 도구

### Claude Code 셸 가드 훅 (`.claude/hooks/guard-shell.mjs`)

`.claude/settings.json` 의 PreToolUse 훅(matcher `Bash|PowerShell`)이 셸 명령을 검사해 차단한다(exit 2).

| 규칙 | 차단 | 대안 |
|---|---|---|
| G1 | 상한 없는 `until`/`while` + `sleep`(PS `Start-Sleep`) 폴링 루프 | `run_in_background` 완료 알림, `timeout 600 bash -c '…'`, `for i in $(seq 1 N)` |
| G2 | 상한 없는 루프 안의 PC 전체 프로세스 조회(`tasklist`·`Get-Process`·`pgrep`·`ps -ef`) | 자기 PID·출력 파일만 조건으로 |
| G3 | `python`/`py` 에 stdin(`-`, heredoc, 인자 없는 파이프) | `.py` 파일 실행, `python -c` |
| G4 | 이름 기반 종료(`taskkill /IM`, `Stop-Process -Name`, `pkill`, `killall`) | PID 로 종료 |

- 시험: `npx vitest run .claude/hooks/guard-shell.test.mjs`(T-hook-01~08, `pnpm test` 에 포함). 첫 줄 `// @vitest-environment node` 는 지우지 않는다. 훅 설정을 바꾸면 세션 재시작 또는 `/hooks` 확인

### WebView 디버깅

- `pnpm tauri dev` 창에서 우클릭 → 검사(DevTools)
- 스모크(CDP): `MAM_DATA_DIR=<scratch> WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=19333" pnpm tauri dev` → `http://127.0.0.1:19333/json` 에서 대상 확인, `window.__TAURI_INTERNALS__.invoke(...)` 호출, 이벤트는 `plugin:event|listen` 구독. 소스가 바뀌면 dev 감시자 재시작으로 연결이 끊긴다

### shadcn/ui · Rust ↔ TypeScript 계약

- `pnpm dlx shadcn@latest add <component>` — 스타일 `radix-vega`(`components.json`). 추가 후 `cn` import 가 `@/lib/utils` 인지 확인하고, `package.json` 에 `cn` 패키지가 추가됐으면 제거
- command 이름·인자·반환 타입은 `src-tauri/src/commands.rs`(및 `*_commands.rs`)와 `src/lib/ipc.ts` 를 함께 수정하고 계약 테스트(`T-ipc-*`, `T-ui-ipc-*`)를 갱신한다. 이벤트 이름(`jobs://progress`, `models://progress`, `ai://analysis`, `export://progress`)·페이로드도 같다
