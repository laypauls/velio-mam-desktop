-- 콘텐츠 제목 (추가형). NULL = 제목 없음(파일명 표시). MAM 메타데이터일 뿐 원본 파일명은 바꾸지 않는다.
-- 이전 버전 앱은 이 컬럼을 무시해도 동작한다.
ALTER TABLE asset_user_data ADD COLUMN title TEXT;
