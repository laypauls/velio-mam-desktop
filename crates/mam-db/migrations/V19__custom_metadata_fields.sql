-- 사용자 정의 메타데이터 필드 (추가형: 새 테이블만). 설계: docs/requests/2026-11/20261115-135644-custom-metadata-fields.md D1
-- 검색은 Tantivy 가 담당하므로 값 컬럼에 검색용 인덱스를 두지 않는다 (재색인·삭제 대상 조회용 인덱스만).
-- 필드 타입 <-> 채워진 값 컬럼 일치, enum 선택지 소속, 필드 수 상한은 테이블 간 규칙이라 저장소 계층(mam-core)이 검증한다.

-- 필드 정의. field_id 는 불변 — AUTOINCREMENT 로 삭제된 id 를 재발급하지 않는다 (파생 인덱스 키 `f{field_id}` 오연결 방지).
-- 타입은 생성 후 바꾸지 않는다. 삭제는 실제 삭제라 archived_at 이 없다.
CREATE TABLE metadata_fields (
    field_id   INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT NOT NULL,
    name_key   TEXT NOT NULL UNIQUE,
    value_type TEXT NOT NULL CHECK (value_type IN ('keyword', 'number', 'date', 'boolean', 'enum', 'multi_enum')),
    position   INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    -- 삭제 요청 시각. NOT NULL 이면 삭제 중(청크 삭제 대기) — 읽기·쓰기에서 없는 필드로 취급하고 name_key 는 sentinel 로 비운다
    deleting_at INTEGER
) STRICT;

-- enum / multi_enum 선택지. option_id 는 multi_enum 값의 ord 로 쓰이므로 재발급하지 않는다 (AUTOINCREMENT).
CREATE TABLE metadata_field_options (
    option_id INTEGER PRIMARY KEY AUTOINCREMENT,
    field_id  INTEGER NOT NULL REFERENCES metadata_fields (field_id) ON DELETE CASCADE,
    label     TEXT NOT NULL,
    label_key TEXT NOT NULL,
    position  INTEGER NOT NULL,
    -- 삭제 요청 시각 (필드와 같은 의미). label_key 는 sentinel 로 비운다
    deleting_at INTEGER,
    UNIQUE (field_id, label_key)
) STRICT;

-- 자산별 값. 단일값 필드는 ord = 0, multi_enum 은 ord = option_id (같은 선택지 중복을 PK 가 막는다).
-- enum/multi_enum 의 option_id 는 value_int 에 있으며 FK 를 걸 수 없다 — 선택지 삭제 시 저장소가 같은 트랜잭션에서 값을 지운다.
CREATE TABLE asset_metadata_values (
    asset_id   INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    field_id   INTEGER NOT NULL REFERENCES metadata_fields (field_id) ON DELETE CASCADE,
    ord        INTEGER NOT NULL CHECK (ord >= 0),
    value_text TEXT,
    value_int  INTEGER,
    value_real REAL CHECK (value_real IS NULL OR (value_real > -9e999 AND value_real < 9e999)),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (asset_id, field_id, ord),
    CHECK ((value_text IS NOT NULL) + (value_int IS NOT NULL) + (value_real IS NOT NULL) = 1)
) STRICT, WITHOUT ROWID;

-- 필드 단위 재색인·필드/선택지 삭제 대상 자산 조회용
CREATE INDEX idx_asset_metadata_values_field ON asset_metadata_values (field_id, asset_id);
