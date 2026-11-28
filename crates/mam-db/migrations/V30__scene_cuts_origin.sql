-- asset_scene_cuts 등록 출처 (추가형): 자동 추출 옵션(scene_detect_auto)을 끄면 자동 등록분의 대기 행만 정리하기 위해 기록한다.
-- NULL(구 행) = manual 취급. 구 앱은 컬럼을 무시하므로 롤백해도 안전하다(db.md §5). PRAGMA 를 쓰지 않는다.
ALTER TABLE asset_scene_cuts ADD COLUMN origin TEXT
    CHECK (origin IS NULL OR origin IN ('auto', 'manual'));
-- V30 이전에 대기(pending) 중이던 행은 자동 등록분으로 간주해 'auto' 로 표시한다 (AI V29 와 같은 판단). 옵션이 꺼진 채 남은
-- 대기분이 업그레이드 뒤 계속 실행되지 않게 하기 위해서다. 드물게 섞인 수동 요청분은 정리되며 다시 요청하면 된다.
-- 완료·실패·미지원 행은 NULL 그대로 둔다.
UPDATE asset_scene_cuts SET origin = 'auto' WHERE state = 'pending' AND origin IS NULL;
