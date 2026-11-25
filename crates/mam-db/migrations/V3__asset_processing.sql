-- P2-3 자산 처리: 메타데이터 확장, 파생물(썸네일·미리보기) 상태, 폴더 재등록 재연결 인덱스. 추가형 변경만 (db.md §5).
-- 설계 근거: docs/requests/2026-10/20261028-053001-p2-3-asset-processing.md

-- V1 media_metadata(width/height/duration_ms/codec/captured_at)에 컬럼을 더한다 (모두 NULL 허용).
--   width/height : 방향·회전을 적용한 표시 크기
--   playback     : WebView2 재생 판정 (playable / conditional / not_playable), 행이 없거나 NULL = 아직 모름
--   extra        : 표시용 추가 메타데이터의 JSON 객체 (컨테이너 등)
ALTER TABLE media_metadata ADD COLUMN audio_codec TEXT;
ALTER TABLE media_metadata ADD COLUMN fps REAL;
ALTER TABLE media_metadata ADD COLUMN bitrate INTEGER;
ALTER TABLE media_metadata ADD COLUMN camera TEXT;
ALTER TABLE media_metadata ADD COLUMN orientation INTEGER;
ALTER TABLE media_metadata ADD COLUMN playback TEXT
    CHECK (playback IS NULL OR playback IN ('playable', 'conditional', 'not_playable'));
ALTER TABLE media_metadata ADD COLUMN extra TEXT;

-- 파생물 처리 상태. 행이 없으면 pending (아직 처리 전). 파일 존재만으로 판단하지 않는다 (D3).
--   meta  : 메타데이터 추출
--   thumb : 320/1024 썸네일 (음성은 unsupported)
--   strip : 영상 미리보기 프레임 스트립 (영상만)
-- error 는 failed 일 때의 사유(경로 포함 가능 - 프론트에 그대로 내보내지 않는다).
CREATE TABLE asset_derivatives (
    asset_id   INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('meta', 'thumb', 'strip')),
    state      TEXT NOT NULL CHECK (state IN ('pending', 'done', 'failed', 'unsupported')),
    error      TEXT,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (asset_id, kind)
) STRICT, WITHOUT ROWID;
-- 실패 목록·수동 재시도
CREATE INDEX idx_asset_derivatives_failed ON asset_derivatives (kind, asset_id) WHERE state = 'failed';

-- D9: 폴더를 제거했다가 다시 등록할 때 detach 된 위치를 같은 볼륨·path_key 로 찾는다.
-- (uq_file_locations_active_path 는 활성 위치만 덮는다)
CREATE INDEX idx_file_locations_detached_path
    ON file_locations (volume_id, path_key) WHERE detached_at IS NOT NULL;
