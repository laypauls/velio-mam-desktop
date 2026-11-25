-- 외부 AI 사용량에 인증 방식을 기록한다 (추가형). 'apiKey' | 'subscription', 기존 행은 NULL(= apiKey).
-- 구독(CLI 대리 실행) 사용량과 API 키 과금 사용량을 사용량 화면에서 구분하기 위함 — 이전 버전 앱은 이 컬럼을 무시해도 동작한다.
ALTER TABLE ai_usage ADD COLUMN auth_mode TEXT;
