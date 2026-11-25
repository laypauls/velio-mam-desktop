-- P2-7 사용자 데이터: 메모·즐겨찾기, 가상 컬렉션. 추가형 변경만 (db.md §5).
-- 태그는 V1 `tags`/`asset_tags` 를 그대로 쓴다 (name_key = NFC + 소문자).
-- 설계 근거: docs/requests/2026-10/20261028-221050-p2-7-8-user-data-reuse-backend.md

-- 자산당 0~1행. 행이 없으면 메모 '' / 즐겨찾기 아님.
CREATE TABLE asset_user_data (
    asset_id   INTEGER PRIMARY KEY REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    memo       TEXT NOT NULL DEFAULT '',
    favorite   INTEGER NOT NULL DEFAULT 0 CHECK (favorite IN (0, 1)),
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_asset_user_data_favorite ON asset_user_data (asset_id) WHERE favorite = 1;

-- 가상 컬렉션 (원본 폴더와 무관). name_key = NFC + 소문자, 중복 불가.
CREATE TABLE collections (
    collection_id INTEGER PRIMARY KEY,
    name          TEXT NOT NULL,
    name_key      TEXT NOT NULL UNIQUE,
    created_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE collection_assets (
    collection_id INTEGER NOT NULL REFERENCES collections (collection_id) ON DELETE CASCADE,
    asset_id      INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    added_at      INTEGER NOT NULL,
    PRIMARY KEY (collection_id, asset_id)
) STRICT;
-- 자산이 속한 컬렉션 조회(색인 문서 구성)와 asset_id FK 검사용
CREATE INDEX idx_collection_assets_asset ON collection_assets (asset_id, collection_id);
