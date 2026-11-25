-- 관리 폴더 숨기기 (추가형): 1 = 검색·전체 보기에서 제외 (수집·분석은 계속). 질의 시점 필터라 색인은 바꾸지 않는다.
ALTER TABLE managed_folders ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1));
