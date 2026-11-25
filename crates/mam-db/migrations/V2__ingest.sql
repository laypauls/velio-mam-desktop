-- P2-2 수집: 관리 폴더, 스캔 실행/오류, 폴더 트리, 작업 대기열. 추가형 변경만 (db.md §5).
-- 설계 근거: docs/requests/2026-10/20261028-052907-p2-2-ingest-scan-watch.md

-- 관리 폴더. rel_root/path_key 는 볼륨 마운트 지점 기준 (드라이브 문자 변경 대응). 볼륨 루트 자체를 등록하면 ''.
CREATE TABLE managed_folders (
    folder_id         INTEGER PRIMARY KEY,
    volume_id         INTEGER NOT NULL REFERENCES volumes (volume_id) ON DELETE RESTRICT,
    rel_root          TEXT NOT NULL,
    path_key          TEXT NOT NULL,
    display_path      TEXT NOT NULL,
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    -- 사용자 제외 하위 경로: 폴더 루트 기준 상대 경로의 JSON 배열
    excluded          TEXT NOT NULL DEFAULT '[]',
    last_full_scan_at INTEGER,
    created_at        INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX uq_managed_folders_root ON managed_folders (volume_id, path_key);

-- 스캔 실행 이력. cursor = 마지막으로 완료한 디렉터리의 폴더 루트 기준 path_key (''=루트), NULL = 아직 없음.
CREATE TABLE scan_runs (
    run_id      INTEGER PRIMARY KEY,
    folder_id   INTEGER NOT NULL REFERENCES managed_folders (folder_id) ON DELETE CASCADE,
    kind        TEXT NOT NULL CHECK (kind IN ('full', 'rescan', 'watch')),
    status      TEXT NOT NULL CHECK (status IN ('running', 'paused', 'done', 'failed', 'cancelled')),
    cursor      TEXT,
    counts      TEXT NOT NULL DEFAULT '{}',
    error_code  TEXT,
    started_at  INTEGER NOT NULL,
    finished_at INTEGER
) STRICT;
CREATE INDEX idx_scan_runs_folder ON scan_runs (folder_id, run_id);

-- 스캔 오류. rel_path = 폴더 루트 기준 표시용 상대 경로('/' 구분, 루트 자체는 '').
CREATE TABLE scan_errors (
    error_id    INTEGER PRIMARY KEY,
    folder_id   INTEGER NOT NULL REFERENCES managed_folders (folder_id) ON DELETE CASCADE,
    rel_path    TEXT NOT NULL,
    error_code  TEXT NOT NULL,
    message     TEXT NOT NULL,
    attempts    INTEGER NOT NULL DEFAULT 1 CHECK (attempts >= 1),
    last_at     INTEGER NOT NULL,
    resolved_at INTEGER
) STRICT;
CREATE UNIQUE INDEX uq_scan_errors_path ON scan_errors (folder_id, rel_path, error_code);
CREATE INDEX idx_scan_errors_open ON scan_errors (error_id) WHERE resolved_at IS NULL;

-- 폴더 탐색 트리(지연 로딩). 스캐너가 유지한다. dir_key/parent_key = 폴더 루트 기준 path_key (루트 직속의 parent_key = '').
CREATE TABLE folder_dirs (
    folder_id  INTEGER NOT NULL REFERENCES managed_folders (folder_id) ON DELETE CASCADE,
    dir_key    TEXT NOT NULL,
    parent_key TEXT NOT NULL,
    name       TEXT NOT NULL,
    rel_path   TEXT NOT NULL,
    seen_at    INTEGER NOT NULL,
    PRIMARY KEY (folder_id, dir_key)
) STRICT, WITHOUT ROWID;
CREATE INDEX idx_folder_dirs_parent ON folder_dirs (folder_id, parent_key, dir_key);

-- 작업 대기열. 성공한 작업은 행을 지운다. 대기 중인 같은 (kind, asset) / (kind, folder) 는 하나만 둔다.
-- asset_id 에 FK 를 두지 않는다 — 작업은 자산보다 먼저 정리될 수 있고 자산은 소프트 삭제다.
CREATE TABLE jobs (
    job_id     INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('scan', 'meta', 'thumb', 'hash', 'index', 'ai')),
    asset_id   INTEGER,
    folder_id  INTEGER REFERENCES managed_folders (folder_id) ON DELETE CASCADE,
    priority   INTEGER NOT NULL DEFAULT 50 CHECK (priority >= 0),
    status     TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'failed')),
    attempts   INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    not_before INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_jobs_ready ON jobs (kind, priority, job_id) WHERE status = 'pending';
CREATE INDEX idx_jobs_kind_status ON jobs (kind, status);
CREATE UNIQUE INDEX uq_jobs_pending_asset ON jobs (kind, asset_id)
    WHERE asset_id IS NOT NULL AND status = 'pending';
CREATE UNIQUE INDEX uq_jobs_pending_folder ON jobs (kind, folder_id)
    WHERE folder_id IS NOT NULL AND status = 'pending';
CREATE INDEX idx_jobs_folder ON jobs (folder_id) WHERE folder_id IS NOT NULL;

-- P1-4 잔여 위험: ScanSession::begin 의 missing 후보 적재가 file_locations 전체 스캔이 되지 않게 한다.
CREATE INDEX idx_file_locations_missing ON file_locations (volume_id)
    WHERE detached_at IS NULL AND presence = 'missing';
