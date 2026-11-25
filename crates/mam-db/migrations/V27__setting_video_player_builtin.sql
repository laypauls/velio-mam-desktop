-- 설정 키 rename (추가형): video_player_vlc -> video_player_builtin. 내장 플레이어가 libVLC 에서 libmpv 로 바뀌어 이름만 정리한다.
-- 옛 행은 지우지 않는다 — 구 앱으로 롤백해도 읽을 수 있어야 한다(db.md §5). 새 코드는 새 키만 읽고 쓴다. PRAGMA 를 쓰지 않는다.
INSERT OR IGNORE INTO settings(key, value) SELECT 'video_player_builtin', value FROM settings WHERE key = 'video_player_vlc';
