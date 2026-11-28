-- V28 이전에 대기·실행 중이던 단계는 자동 등록분으로 간주해 'background' 로 표시한다. 옵션을 꺼도 계속 돌던 결함의
-- 피해 사용자가 업그레이드 후 옵션이 꺼진 채로 남은 작업(최대 수백 자산)을 계속 실행하지 않게 하기 위해서다.
-- 드물게 섞인 수동 요청분은 정리되며 다시 요청하면 된다. 완료·실패·건너뜀 행은 NULL 그대로 둔다. PRAGMA 를 쓰지 않는다.
UPDATE ai_task_state SET origin = 'background' WHERE state IN ('queued', 'running') AND origin IS NULL;
