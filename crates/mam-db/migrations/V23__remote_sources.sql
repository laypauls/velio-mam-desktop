-- 원격 스토리지 접속(SSH, 사용자가 명시적으로 만든 것만)과 원격 피드 커서 (추가형 — 새 테이블 2개).
-- 비밀(비밀번호·개인 키)은 평문으로 두지 않는다: auth_blob 은 Windows DPAPI(CurrentUser) 암호문이다.
CREATE TABLE remote_sources (
    source_id  INTEGER PRIMARY KEY,
    -- 'ddp_audit' = DDP(AVFS) 감사 로그 이어 읽기, 'ses_queue' = storage-event-service SQLite 큐 (후속 단계, 지금은 값만 예약)
    kind       TEXT    NOT NULL CHECK (kind IN ('ddp_audit', 'ses_queue')),
    name       TEXT    NOT NULL UNIQUE CHECK (length(name) BETWEEN 1 AND 100),
    host       TEXT    NOT NULL CHECK (length(host) BETWEEN 1 AND 253),
    port       INTEGER NOT NULL DEFAULT 22 CHECK (port BETWEEN 1 AND 65535),
    user       TEXT    NOT NULL CHECK (length(user) BETWEEN 1 AND 128),
    -- 'password' | 'key' (개인 키 PEM 본문 — 파일 경로가 아니다: 원본 파일 시스템 밖의 값을 앱이 직접 쥔다)
    auth_kind  TEXT    NOT NULL CHECK (auth_kind IN ('password', 'key')),
    auth_blob  BLOB    NOT NULL,
    -- 사용자가 확인한 호스트 키 지문 ("SHA256:…"). NULL = 아직 확인 전 — 이 상태에서는 접속 시험만 되고 감시용 접속은 거부한다
    host_key   TEXT    NULL,
    -- ddp_audit: 감사 로그 폴더 (NULL = 기본 /tmp/avfs-dbase/audit) / ses_queue: events.db 경로
    log_dir    TEXT    NULL,
    db_path    TEXT    NULL,
    -- 읽기 간격(초)
    poll_sec   INTEGER NOT NULL DEFAULT 3 CHECK (poll_sec BETWEEN 1 AND 3600),
    -- [{"from":"/","to":"F:\\UHD"}] — 볼륨(컨테이너) 경로 -> 이 PC 경로
    path_map_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;

-- 원격 피드 커서: ddp_audit = 파일별 바이트 오프셋 JSON, ses_queue = 마지막 id.
-- 변경 대기열(watch_pending)에 행을 쓴 **같은 트랜잭션**에서 갱신한다 — 앱이 꺼졌다 켜져도 놓친 줄부터 이어 읽는다.
CREATE TABLE watch_cursors (
    source_id  INTEGER PRIMARY KEY REFERENCES remote_sources (source_id) ON DELETE CASCADE,
    cursor     TEXT    NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
