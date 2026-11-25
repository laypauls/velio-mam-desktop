-- DRAFT: 첫 외부 빌드 전 동결 예정, 수정 시 개발 DB 재생성
--
-- P1-1 코어 스키마. 규칙:
--   * 모든 테이블 STRICT, 시각은 UTC Unix ms INTEGER, 열거값은 TEXT + CHECK
--   * 연결 PRAGMA(foreign_keys/WAL/busy_timeout)는 연결 함수가 설정한다 (마이그레이션 트랜잭션 안에서 PRAGMA 불가)
--   * 설계 근거: docs/requests/2026-10/20261024-234145-p1-1-data-model-schema.md

-- 볼륨: 드라이브 문자 변경 대응. 위치의 경로는 볼륨 기준 상대 경로.
-- availability = offline 은 "볼륨 연결 끊김"만 의미한다.
CREATE TABLE volumes (
    volume_id        INTEGER PRIMARY KEY,
    kind             TEXT NOT NULL CHECK (kind IN ('fixed', 'removable', 'network')),
    volume_key       TEXT NOT NULL UNIQUE,
    label            TEXT,
    last_mount_point TEXT,
    availability     TEXT NOT NULL DEFAULT 'online' CHECK (availability IN ('online', 'offline')),
    last_seen_at     INTEGER NOT NULL
) STRICT;

-- 자산: 파일마다 별도 자산 (D1), 내용이 바뀌면 새 자산 (D2).
-- AUTOINCREMENT: 삭제된 asset_id 를 재발급하지 않는다 (파생 인덱스의 옛 문서 오연결 방지).
CREATE TABLE media_assets (
    asset_id     INTEGER PRIMARY KEY AUTOINCREMENT,
    media_type   TEXT NOT NULL CHECK (media_type IN ('image', 'video', 'audio')),
    status       TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'superseded', 'deleted')),
    content_hash BLOB,
    hash_algo    TEXT CHECK (hash_algo IN ('blake3')),
    hash_state   TEXT NOT NULL DEFAULT 'pending' CHECK (hash_state IN ('pending', 'done', 'failed')),
    file_size    INTEGER NOT NULL CHECK (file_size >= 0),
    modified_at  INTEGER NOT NULL,
    revision     INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    CHECK ((hash_state = 'done') = (content_hash IS NOT NULL)),
    CHECK ((hash_algo IS NULL) = (content_hash IS NULL)),
    -- blake3 digest = 32 bytes (알고리즘을 추가하면 이 CHECK 를 새 마이그레이션으로 확장한다)
    CHECK (content_hash IS NULL OR (hash_algo = 'blake3' AND length(content_hash) = 32))
) STRICT;

-- 중복 비교는 (hash_algo, content_hash) 쌍으로 (D3)
CREATE INDEX idx_media_assets_hash
    ON media_assets (hash_algo, content_hash) WHERE content_hash IS NOT NULL;
CREATE INDEX idx_media_assets_type_status ON media_assets (media_type, status);
CREATE INDEX idx_media_assets_hash_pending
    ON media_assets (hash_state) WHERE hash_state = 'pending';

-- 파일 위치. rel_path 는 표시용 원래 표기, path_key 는 비교용(Rust path_key() 로 생성, 비어 있을 수 없음).
-- presence = missing 은 "볼륨은 연결됐는데 파일 없음"(외부 삭제)만 의미한다. 오프라인은 volumes 와 조인해 계산한다.
-- RESTRICT: 위치 이력이 있는 자산/볼륨을 실수로 지우지 못하게 한다 (자산 삭제는 소프트 삭제).
CREATE TABLE file_locations (
    location_id   INTEGER PRIMARY KEY,
    asset_id      INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE RESTRICT,
    volume_id     INTEGER NOT NULL REFERENCES volumes (volume_id) ON DELETE RESTRICT,
    rel_path      TEXT NOT NULL,
    path_key      TEXT NOT NULL CHECK (path_key <> ''),
    file_identity TEXT,
    presence      TEXT NOT NULL DEFAULT 'present' CHECK (presence IN ('present', 'missing')),
    detached_at   INTEGER,
    last_seen_at  INTEGER NOT NULL
) STRICT;

-- 같은 경로에는 활성 위치가 하나 — 다른 파일이 오면 이전 위치를 detach 한다
CREATE UNIQUE INDEX uq_file_locations_active_path
    ON file_locations (volume_id, path_key) WHERE detached_at IS NULL;
-- 자산당 활성 위치 1개 (D1)
CREATE UNIQUE INDEX uq_file_locations_active_asset
    ON file_locations (asset_id) WHERE detached_at IS NULL;
CREATE INDEX idx_file_locations_asset ON file_locations (asset_id);
-- FK 자식 인덱스: 볼륨 삭제/오프라인 처리 시 해당 볼륨 위치 조회
CREATE INDEX idx_file_locations_volume ON file_locations (volume_id);
CREATE INDEX idx_file_locations_identity
    ON file_locations (file_identity) WHERE file_identity IS NOT NULL;

-- 신·구 자산 관계 (D2)
CREATE TABLE asset_relations (
    from_asset_id INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE RESTRICT,
    to_asset_id   INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE RESTRICT,
    relation      TEXT NOT NULL CHECK (relation IN ('content_changed')),
    created_at    INTEGER NOT NULL,
    PRIMARY KEY (from_asset_id, to_asset_id, relation),
    CHECK (from_asset_id <> to_asset_id)
) STRICT;
-- 역방향 조회("이 자산을 대체한 구자산") 및 to_asset_id FK 검사용
CREATE INDEX idx_asset_relations_to ON asset_relations (to_asset_id);

CREATE TABLE media_metadata (
    asset_id    INTEGER PRIMARY KEY REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    width       INTEGER,
    height      INTEGER,
    duration_ms INTEGER,
    codec       TEXT,
    captured_at INTEGER
) STRICT;

CREATE TABLE tags (
    tag_id   INTEGER PRIMARY KEY,
    name     TEXT NOT NULL,
    name_key TEXT NOT NULL UNIQUE
) STRICT;

CREATE TABLE asset_tags (
    asset_id   INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tags (tag_id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (asset_id, tag_id)
) STRICT;
CREATE INDEX idx_asset_tags_tag ON asset_tags (tag_id, asset_id);

-- Transactional Outbox. asset_id 에 FK 를 두지 않는다 — 자산 행이 삭제돼도 delete 이벤트가
-- 막히거나 사라지면 안 된다 (파생 인덱스 쪽 상태는 자산보다 오래 산다).
-- priority: 작을수록 먼저 처리 (0 = 사용자 편집, 1 = 대량 AI 분석 등 저우선)
CREATE TABLE search_outbox (
    event_id   INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id   INTEGER NOT NULL,
    revision   INTEGER NOT NULL CHECK (revision >= 1),
    event_type TEXT NOT NULL CHECK (event_type IN ('upsert', 'delete')),
    priority   INTEGER NOT NULL DEFAULT 1 CHECK (priority >= 0),
    status     TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'done', 'failed')),
    attempts   INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error TEXT,
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_search_outbox_pending ON search_outbox (status, priority, event_id);

-- 인덱스별 반영된 revision (역전 방지 기준). asset_id 에 FK 없음 — 위와 같은 이유.
CREATE TABLE index_progress (
    index_name       TEXT NOT NULL,
    asset_id         INTEGER NOT NULL,
    applied_revision INTEGER NOT NULL CHECK (applied_revision >= 0),
    PRIMARY KEY (index_name, asset_id)
) STRICT;

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) STRICT;
