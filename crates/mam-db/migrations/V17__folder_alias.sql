-- 관리 폴더 별칭 (추가형): 사이드바·목록에 경로 대신 보여줄 사용자 지정 이름. NULL = 별칭 없음. 표시 전용이라 색인·revision 과 무관.
ALTER TABLE managed_folders ADD COLUMN alias TEXT NULL;
