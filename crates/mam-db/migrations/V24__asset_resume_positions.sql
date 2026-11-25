-- 이어보기 위치 (추가형, 자산 단위). 마지막 재생 위치 1건만 둔다.
-- 검색 색인 대상이 아니므로 revision·search_outbox 를 쓰지 않는다. 원본 미디어와 무관한 MAM 메타데이터
CREATE TABLE asset_resume_positions (
    asset_id    INTEGER PRIMARY KEY REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    position_ms INTEGER NOT NULL CHECK (position_ms >= 0),
    duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms > 0),
    updated_at  INTEGER NOT NULL
) STRICT;
