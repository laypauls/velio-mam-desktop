-- 관리 폴더별 변경 감지 방식·재스캔 일정 정책 (추가형). 기존 폴더는 모두 NULL = 현재 동작(자동 감지 + 전역 rescan_interval_hours).
-- 값 검증은 앱(감지기·정책 레지스트리)이 한다 — 새 감지기·트리거 추가 때 마이그레이션이 필요 없도록 CHECK 를 두지 않는다.
-- 변경 감지 방식 ID ("os_events", "scan_only" …). NULL = 자동
ALTER TABLE managed_folders ADD COLUMN watch_mode TEXT NULL;
-- 감지기별 옵션 JSON 객체 (키는 camelCase). NULL = 기본값
ALTER TABLE managed_folders ADD COLUMN watch_options TEXT NULL;
-- 재스캔 일정 정책 JSON. NULL = 전역 기본(rescan_interval_hours). 해석 불가 값은 전역 기본으로 동작 + 경고 로그
ALTER TABLE managed_folders ADD COLUMN rescan_policy TEXT NULL;
