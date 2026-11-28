-- ai_task_state 등록 출처 (추가형): 자동 옵션을 끄면 그 출처의 대기 단계만 정리하기 위해 기록한다.
-- NULL(구 행) = manual 취급. 구 앱은 컬럼을 무시하므로 롤백해도 안전하다(db.md §5). PRAGMA 를 쓰지 않는다.
ALTER TABLE ai_task_state ADD COLUMN origin TEXT
    CHECK (origin IS NULL OR origin IN ('auto_new', 'background', 'backfill', 'manual'));
