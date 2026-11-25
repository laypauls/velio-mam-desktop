-- 관리 폴더별 스캔 대상 유형 (추가형): 1 = 스캔 대상, 0 = 제외. 기존 폴더는 전부 1 (현재 동작 유지).
-- 최소 1개 유형이 켜져 있어야 한다는 규칙은 코어(set_folder_media_types / add_folder)가 검증한다.
ALTER TABLE managed_folders ADD COLUMN scan_image INTEGER NOT NULL DEFAULT 1 CHECK (scan_image IN (0, 1));
ALTER TABLE managed_folders ADD COLUMN scan_video INTEGER NOT NULL DEFAULT 1 CHECK (scan_video IN (0, 1));
ALTER TABLE managed_folders ADD COLUMN scan_audio INTEGER NOT NULL DEFAULT 1 CHECK (scan_audio IN (0, 1));
