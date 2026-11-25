-- 앞 4K + 뒤 4K 체크섬 (추가형). NULL = 아직 계산 전. 같은 크기·같은 edge_sig 자산만 전체 해시 대상이 된다 (후보 축소용 —
-- 완전 중복 판정은 여전히 content_hash). 이전 버전 앱은 이 컬럼을 무시해도 동작한다.
ALTER TABLE media_assets ADD COLUMN edge_sig BLOB CHECK (edge_sig IS NULL OR length(edge_sig) = 32);
-- 같은 크기·같은 edge_sig 의 활성 자산 조회
CREATE INDEX idx_media_assets_edge ON media_assets (file_size, edge_sig) WHERE status = 'active' AND edge_sig IS NOT NULL;
