-- V6: AI 결과 — STT 전사(P3-4), 임베딩 메타(P3-5), AI 분석·작업 상태·이력·사용량(P3-1/P3-3). 추가형.

-- 로컬 STT 전사 (자산당 1건, 재전사는 덮어쓰기). 원문 언어 보존, AI 설명과 분리.
CREATE TABLE transcripts (
    transcript_id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id      INTEGER NOT NULL UNIQUE REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    language      TEXT NOT NULL DEFAULT '',
    text          TEXT NOT NULL,
    engine        TEXT NOT NULL,
    model         TEXT NOT NULL,
    created_at    INTEGER NOT NULL
) STRICT;

CREATE TABLE transcript_segments (
    segment_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    transcript_id INTEGER NOT NULL REFERENCES transcripts (transcript_id) ON DELETE CASCADE,
    seq           INTEGER NOT NULL CHECK (seq >= 0),
    start_ms      INTEGER NOT NULL CHECK (start_ms >= 0),
    end_ms        INTEGER NOT NULL,
    text          TEXT NOT NULL,
    CHECK (end_ms >= start_ms)
) STRICT;
CREATE UNIQUE INDEX idx_transcript_segments_order ON transcript_segments (transcript_id, seq);

-- 임베딩 메타 (벡터 본문은 raw.f16, 파생 인덱스는 재구축 가능)
CREATE TABLE embeddings (
    embedding_id  INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id      INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    segment_id    INTEGER,
    vector_kind   TEXT NOT NULL CHECK (vector_kind IN ('visual', 'semantic')),
    source        TEXT NOT NULL CHECK (source IN ('image', 'frame', 'text')),
    frame_index   INTEGER,
    model_id      TEXT NOT NULL,
    model_version TEXT NOT NULL,
    dimension     INTEGER NOT NULL CHECK (dimension > 0),
    raw_offset    INTEGER NOT NULL CHECK (raw_offset >= 0),
    created_at    INTEGER NOT NULL,
    superseded_at INTEGER
) STRICT;
CREATE INDEX idx_embeddings_model ON embeddings (vector_kind, model_id, model_version, superseded_at);
CREATE INDEX idx_embeddings_asset ON embeddings (asset_id);

-- AI 분석 결과 (자산당 1건, 재분석은 요청한 종류의 필드를 덮어쓴다). STT 전사(transcripts)와 분리.
CREATE TABLE ai_analyses (
    asset_id    INTEGER PRIMARY KEY REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    description TEXT NOT NULL DEFAULT '',
    keywords    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(keywords)),
    ocr_text    TEXT NOT NULL DEFAULT '',
    summary     TEXT NOT NULL DEFAULT '',
    language    TEXT NOT NULL DEFAULT '',
    provider    TEXT NOT NULL,
    model       TEXT NOT NULL,
    edited      INTEGER NOT NULL DEFAULT 0 CHECK (edited IN (0, 1)),
    analyzed_at INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
) STRICT;

-- 재분석이 덮어쓴 사용자 편집본 보존 (복구 가능하도록)
CREATE TABLE ai_analysis_replaced (
    replaced_id INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id    INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    description TEXT NOT NULL,
    keywords    TEXT NOT NULL,
    ocr_text    TEXT NOT NULL,
    summary     TEXT NOT NULL,
    language    TEXT NOT NULL,
    replaced_at INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_ai_analysis_replaced_asset ON ai_analysis_replaced (asset_id, replaced_id);

-- 단계별 상태 (UI AnalysisStatus 의 원천). 행 없음 = none. queued 행 = 작업이 처리할 단계 목록.
CREATE TABLE ai_task_state (
    asset_id   INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('describe', 'keywords', 'ocr', 'summary', 'stt', 'embedding')),
    state      TEXT NOT NULL CHECK (state IN ('queued', 'running', 'done', 'failed', 'skipped')),
    error_code TEXT,
    attempts   INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (asset_id, kind)
) STRICT, WITHOUT ROWID;
CREATE INDEX idx_ai_task_state_kind_state ON ai_task_state (kind, state, asset_id);

-- 파일별 성공/실패 이력 (message 는 정제된 사용자 메시지만 - 공급자 응답 본문·키 금지)
CREATE TABLE analysis_runs (
    run_id     INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id   INTEGER NOT NULL REFERENCES media_assets (asset_id) ON DELETE CASCADE,
    kind       TEXT NOT NULL CHECK (kind IN ('describe', 'keywords', 'ocr', 'summary', 'stt', 'embedding')),
    provider   TEXT NOT NULL,
    ok         INTEGER NOT NULL CHECK (ok IN (0, 1)),
    error_code TEXT,
    message    TEXT NOT NULL DEFAULT '',
    attempts   INTEGER NOT NULL CHECK (attempts >= 0),
    at         INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_analysis_runs_asset ON analysis_runs (asset_id, at DESC, run_id DESC);

-- 외부 AI 사용량 (요청 1건 = 1행). 자산 삭제 후에도 사용량은 남긴다.
CREATE TABLE ai_usage (
    usage_id      INTEGER PRIMARY KEY AUTOINCREMENT,
    provider      TEXT NOT NULL,
    model         TEXT NOT NULL,
    kind          TEXT NOT NULL,
    asset_id      INTEGER REFERENCES media_assets (asset_id) ON DELETE SET NULL,
    input_tokens  INTEGER NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    output_tokens INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    request_bytes INTEGER NOT NULL DEFAULT 0 CHECK (request_bytes >= 0),
    images        INTEGER NOT NULL DEFAULT 0 CHECK (images >= 0),
    ok            INTEGER NOT NULL CHECK (ok IN (0, 1)),
    error_code    TEXT,
    at            INTEGER NOT NULL
) STRICT;
CREATE INDEX idx_ai_usage_at ON ai_usage (at, provider, model);
CREATE INDEX idx_ai_usage_asset ON ai_usage (asset_id) WHERE asset_id IS NOT NULL;
