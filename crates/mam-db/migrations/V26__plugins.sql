-- 플러그인 레지스트리 · KV · provider 작업 상태 (추가형) + jobs.kind CHECK 에 'plugin' 추가 — db.md §5 "추가형만" 의 예외
-- (2026-11-22 사용자 승인 Q2, V16 과 같은 "값 추가만"). PRAGMA 를 쓰지 않는다 — FK OFF·트랜잭션·foreign_key_check 는 러너가 처리.
-- 검색 색인 대상이 아니므로 revision·search_outbox 를 쓰지 않는다 (태그 기록은 기존 asset_tags + outbox 경로).
-- 정수 키(plugin_ref·provider_ref)는 이 DB 안에서만 의미가 있다 — 설정·파일·IPC 에 쓰지 않는다. 레지스트리 행은 지우지 않는다.

CREATE TABLE plugins (
    plugin_ref INTEGER PRIMARY KEY,
    plugin_id  TEXT NOT NULL UNIQUE CHECK (plugin_id <> '' AND instr(plugin_id, ':') = 0)
) STRICT;

CREATE TABLE plugin_providers (
    provider_ref INTEGER PRIMARY KEY,
    plugin_ref   INTEGER NOT NULL REFERENCES plugins (plugin_ref),
    name         TEXT NOT NULL CHECK (name <> ''),
    UNIQUE (plugin_ref, name),
    UNIQUE (plugin_ref, provider_ref)
) STRICT;

-- asset_id NULL = 플러그인 전역 키. value 는 JSON 텍스트 — 크기(≤64 KiB 제안)·JSON 검증·총량(Q8)은 앱이 검사한다
CREATE TABLE plugin_kv (
    plugin_ref INTEGER NOT NULL REFERENCES plugins (plugin_ref),
    asset_id   INTEGER REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    key        TEXT NOT NULL CHECK (key <> ''),
    value      TEXT NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE UNIQUE INDEX uq_plugin_kv_global ON plugin_kv (plugin_ref, key) WHERE asset_id IS NULL;
CREATE UNIQUE INDEX uq_plugin_kv_asset ON plugin_kv (plugin_ref, asset_id, key) WHERE asset_id IS NOT NULL;
CREATE INDEX idx_plugin_kv_asset ON plugin_kv (asset_id);

-- (plugin, provider, asset) 당 1행. 완료 행도 남긴다 — done_revision 이 역전 방지 기준
CREATE TABLE plugin_tasks (
    task_id         INTEGER PRIMARY KEY,
    plugin_ref      INTEGER NOT NULL,
    provider_ref    INTEGER NOT NULL,
    asset_id        INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    target_revision INTEGER NOT NULL CHECK (target_revision >= 1),
    done_revision   INTEGER NOT NULL DEFAULT 0 CHECK (done_revision >= 0),
    status          TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'done', 'failed')),
    attempts        INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error      TEXT,
    updated_at      INTEGER NOT NULL,
    FOREIGN KEY (plugin_ref, provider_ref) REFERENCES plugin_providers (plugin_ref, provider_ref)
) STRICT;
CREATE UNIQUE INDEX uq_plugin_tasks_key ON plugin_tasks (plugin_ref, provider_ref, asset_id);
CREATE INDEX idx_plugin_tasks_pending ON plugin_tasks (plugin_ref, task_id) WHERE status = 'pending';
CREATE INDEX idx_plugin_tasks_failed ON plugin_tasks (plugin_ref, task_id) WHERE status = 'failed';
CREATE INDEX idx_plugin_tasks_asset ON plugin_tasks (asset_id);

-- jobs 재생성: V16 과 열 순서·이름·제약·인덱스 동일, kind 에 'plugin' 만 추가
CREATE TABLE jobs_new (
    job_id     INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('scan', 'meta', 'thumb', 'hash', 'index', 'ai', 'scene', 'plugin')),
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
INSERT INTO jobs_new (job_id, kind, asset_id, folder_id, priority, status, attempts, not_before, last_error, created_at, updated_at)
SELECT job_id, kind, asset_id, folder_id, priority, status, attempts, not_before, last_error, created_at, updated_at FROM jobs;
DROP TABLE jobs;
ALTER TABLE jobs_new RENAME TO jobs;
CREATE INDEX idx_jobs_ready ON jobs (kind, priority, job_id) WHERE status = 'pending';
CREATE INDEX idx_jobs_kind_status ON jobs (kind, status);
CREATE UNIQUE INDEX uq_jobs_pending_asset ON jobs (kind, asset_id)
    WHERE asset_id IS NOT NULL AND status = 'pending';
CREATE UNIQUE INDEX uq_jobs_pending_folder ON jobs (kind, folder_id)
    WHERE folder_id IS NOT NULL AND status = 'pending';
CREATE INDEX idx_jobs_folder ON jobs (folder_id) WHERE folder_id IS NOT NULL;
