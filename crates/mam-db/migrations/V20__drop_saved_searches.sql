-- 저장한 검색 기능 제거: V5 의 `saved_searches` 테이블을 삭제한다. `recent_searches` 는 유지한다.
-- 설계 근거: docs/requests/2026-11/20261115-210058-remove-saved-search.md (D3)
-- db.md §5(추가형만 허용)의 예외 — 사용자가 승인했다. FK·인덱스·참조하는 테이블이 없고, 저장된 행은 영구 삭제된다
-- (적용 전 백업에는 남는다). 이미 적용된 V5 는 수정하지 않는다.

DROP TABLE saved_searches;
