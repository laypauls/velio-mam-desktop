-- ai_task_state 파생물(썸네일·프레임 스트립) 대기 시작 시각 (추가형): 대기 상한(30분) 초과를 판정하기 위해 첫 대기 시각을 기록한다.
-- updated_at 은 등록(큐 대기) 시각이라 앞선 작업을 기다린 시간이 섞여 첫 대기 시각으로 쓸 수 없다.
-- NULL = 대기한 적 없음(구 행 포함). 구 앱은 컬럼을 무시하므로 롤백해도 안전하다(db.md §5). PRAGMA 를 쓰지 않는다.
ALTER TABLE ai_task_state ADD COLUMN wait_since INTEGER;
