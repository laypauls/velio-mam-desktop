-- 활성 임베딩 맵 로드(`ActiveMapCache::load_full`)·활성 raw 읽기·raw 하한(MAX(raw_offset)) 조회용 커버링 인덱스 (추가형).
-- 기존 idx_embeddings_model 은 asset_id·dimension·raw_offset 을 담지 않아 행마다 테이블 조회가 필요했다
-- (220만 행 로드 1.3~1.7초). 모델 ID·버전이 차원을 정하지만 질의가 dimension 도 거르므로 인덱스에 포함한다.
-- superseded_at 다음에 embedding_id 가 와서 활성 구간이 이미 embedding_id 순이다 (ORDER BY 정렬 불필요).
CREATE INDEX idx_embeddings_active_map ON embeddings (
    vector_kind, model_id, model_version, dimension, superseded_at, embedding_id, asset_id, raw_offset
);
