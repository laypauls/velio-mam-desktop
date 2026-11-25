-- jobs.kind CHECK 에 'scene' 추가 — db.md §5 "추가형만" 의 예외 (2026-11-13 사용자 승인 D2).
-- 12단계 재생성: FK OFF·트랜잭션은 mam-db 러너가 처리하므로 PRAGMA 를 쓰지 않는다. 열 순서·이름·제약·인덱스는 V2 와 동일하게 유지
CREATE TABLE jobs_new (
    job_id     INTEGER PRIMARY KEY,
    kind       TEXT NOT NULL CHECK (kind IN ('scan', 'meta', 'thumb', 'hash', 'index', 'ai', 'scene')),
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
