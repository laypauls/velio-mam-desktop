-- 관리 폴더별 이어보기 사용 여부 (추가형). 1 = 사용(기본), 0 = 사용 안 함.
-- 구 앱은 이 컬럼을 무시하므로 롤백해도 동작한다(그때는 전부 사용으로 동작).
ALTER TABLE managed_folders ADD COLUMN resume_enabled INTEGER NOT NULL DEFAULT 1 CHECK (resume_enabled IN (0, 1));
