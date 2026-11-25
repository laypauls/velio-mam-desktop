-- 재생 구간 (추가형, 자산 단위). 저장 시 정규화(정렬·병합)된, 서로 겹치거나 맞닿지 않는 구간만 둔다.
-- 검색 색인 대상이 아니므로 revision·search_outbox 를 쓰지 않는다. 원본 미디어와 무관한 MAM 메타데이터
CREATE TABLE asset_play_ranges (
    asset_id INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    start_ms INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms   INTEGER NOT NULL CHECK (end_ms > start_ms),
    PRIMARY KEY (asset_id, start_ms)
) STRICT, WITHOUT ROWID;
