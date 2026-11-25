-- 감시가 보고한 "확인할 경로" 영속 대기열 (추가형). 경로당 1행으로 합치고(UPSERT), 반영하면 삭제한다.
-- SQLite 가 원천인 작업 상태이며 파생 인덱스가 아니다 — 앱이 꺼져도 보고된 변경을 잃지 않는다. 폴더 제거 시 CASCADE 로 함께 지운다.
CREATE TABLE watch_pending (
    folder_id     INTEGER NOT NULL REFERENCES managed_folders (folder_id) ON DELETE CASCADE,
    -- 정규화 키 (스캔과 같은 mam_db::path_key — 볼륨 루트부터의 절대 경로 기준)
    path_key      TEXT    NOT NULL,
    abs_path      TEXT    NOT NULL,
    kind          TEXT    NOT NULL CHECK (kind IN ('path', 'subtree')),
    -- 보고한 쪽 (감지기 ID 또는 'scan') — 진단용
    source        TEXT    NOT NULL,
    first_seen_at INTEGER NOT NULL,
    -- 디바운스 기준: 같은 경로가 다시 보고되면 앞으로 밀린다
    last_seen_at  INTEGER NOT NULL,
    -- 다음 확인 시각 (디바운스 끝 · 다음 표본 · 재시도 백오프)
    due_at        INTEGER NOT NULL,
    -- 복사 완료 판정 표본 (크기·수정 시각 ms). 연속으로 같은 횟수가 stable_count
    sample_size   INTEGER,
    sample_mtime  INTEGER,
    stable_count  INTEGER NOT NULL DEFAULT 0,
    attempts      INTEGER NOT NULL DEFAULT 0,
    last_error    TEXT,
    PRIMARY KEY (folder_id, path_key)
) STRICT, WITHOUT ROWID;

CREATE INDEX idx_watch_pending_due ON watch_pending (due_at);
