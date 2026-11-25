-- 동영상 장면전환점 (추가형, 자산당 1행 — D1). 검색 색인 대상이 아니므로 revision·search_outbox 를 쓰지 않는다.
-- 행 없음 = 추출 요청된 적 없음. 컷 배열 BLOB = [버전 0x01] + u32 LE ms 엄격 오름차순, 빈 배열은 NULL.
-- sensitivity·error 는 값이 늘어날 수 있어 CHECK 를 두지 않는다 (검증은 앱). PRAGMA 쓰지 않음 (러너가 처리)
CREATE TABLE asset_scene_cuts (
    asset_id              INTEGER PRIMARY KEY REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    state                 TEXT NOT NULL CHECK (state IN ('pending', 'done', 'failed', 'unsupported')),
    sensitivity           TEXT,
    detector_version      INTEGER CHECK (detector_version IS NULL OR detector_version >= 1),
    requested_sensitivity TEXT,
    auto_cuts             BLOB,
    user_cuts             BLOB,
    suppressed            BLOB,
    error                 TEXT,
    updated_at            INTEGER NOT NULL
) STRICT;
-- 볼륨 재연결·시작 시 pending 인데 작업이 없는 행을 다시 등록한다
CREATE INDEX idx_asset_scene_cuts_pending ON asset_scene_cuts (asset_id) WHERE state = 'pending';
