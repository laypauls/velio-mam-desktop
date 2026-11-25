-- 콘텐츠 별점 0~5 (추가형). 0 = 미평가 단일 상태. 이전 버전 앱은 이 컬럼을 무시해도 동작한다.
ALTER TABLE asset_user_data ADD COLUMN rating INTEGER NOT NULL DEFAULT 0 CHECK (rating BETWEEN 0 AND 5);
CREATE INDEX idx_asset_user_data_rating ON asset_user_data (rating) WHERE rating > 0;
