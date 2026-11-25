-- P2-5 검색 이력: 최근 검색(최대 50개 유지), 저장된 검색. 추가형 변경만 (db.md §5).
-- 설계 근거: docs/requests/2026-10/20261028-221709-p2-5-search-coordinator.md (D8)

-- 최근 검색. 같은 (query, filters_json) 조건은 한 행을 갱신한다. filters_json 은 Rust 가 직렬화한 정규 형태.
CREATE TABLE recent_searches (
    recent_id    INTEGER PRIMARY KEY,
    query        TEXT NOT NULL,
    filters_json TEXT NOT NULL,
    used_at      INTEGER NOT NULL,
    UNIQUE (query, filters_json)
) STRICT;
-- 최신순 조회와 오래된 행 정리
CREATE INDEX idx_recent_searches_used ON recent_searches (used_at, recent_id);

-- 저장된 검색. name_key = NFC + 소문자, 중복 불가.
CREATE TABLE saved_searches (
    saved_id     INTEGER PRIMARY KEY,
    name         TEXT NOT NULL,
    name_key     TEXT NOT NULL UNIQUE,
    query        TEXT NOT NULL,
    filters_json TEXT NOT NULL,
    sort_json    TEXT NOT NULL,
    created_at   INTEGER NOT NULL
) STRICT;
