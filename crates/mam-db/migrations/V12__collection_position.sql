-- 컬렉션 안 순서 (추가형). NULL = 이전 앱이 추가한 행 → 위치 있는 행 뒤, added_at·asset_id 순. 다음 쓰기 때 정규화
ALTER TABLE collection_assets ADD COLUMN position INTEGER;
UPDATE collection_assets SET position = r.rn * 1048576
  FROM (SELECT collection_id, asset_id,
               row_number() OVER (PARTITION BY collection_id ORDER BY added_at, asset_id) AS rn
          FROM collection_assets) AS r
 WHERE collection_assets.collection_id = r.collection_id AND collection_assets.asset_id = r.asset_id;
CREATE INDEX idx_collection_assets_position ON collection_assets (collection_id, position);
