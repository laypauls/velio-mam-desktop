//! Tantivy 스키마 — 기술 검토서 §5 필드 표를 조정한 것.
//!
//! 조정: `availability` 대신 `volume_id` 색인(결정 D2), 파일명 부분 일치 전략용
//! `file_name_ngram`(B)·`file_name_raw`(C) 필드 추가, 확장자 분리(`ext`).
//! 저장(stored)은 `asset_id`·`revision` 뿐이다 — 상세는 SQLite 에서 조회한다.

use tantivy::schema::{
    FAST, Field, INDEXED, IndexRecordOption, JsonObjectOptions, STORED, STRING, Schema,
    TextFieldIndexing, TextOptions,
};

use crate::analyzer::{FILENAME, FILENAME_NGRAM, FILENAME_RAW, KOREAN};

/// 스키마의 필드 핸들 모음.
#[derive(Clone, Copy, Debug)]
pub struct Fields {
    pub asset_id: Field,
    pub revision: Field,
    pub file_name: Field,
    pub file_name_ngram: Field,
    pub file_name_raw: Field,
    pub ext: Field,
    pub user_tags: Field,
    pub ai_keywords: Field,
    pub ai_description: Field,
    pub user_memo: Field,
    /// 사용자 지정 제목 (검색용 텍스트)
    pub title: Field,
    pub transcript: Field,
    pub media_type: Field,
    pub volume_id: Field,
    pub presence: Field,
    pub captured_at: Field,
    /// 유닉스 초 — 촬영일이 있으면 촬영일, 없으면 수정일(ms/1000). 설정 `capturedAtFallback=modified` 일 때 질의가 쓴다
    pub captured_or_modified: Field,
    pub file_size: Field,
    pub duration_ms: Field,
    /// 즐겨찾기 0/1 (P2-7)
    pub favorite: Field,
    /// 별점 0..=5 (0 = 미평가)
    pub rating: Field,
    /// 다중 값: 소속 컬렉션 id (P2-7)
    pub collection_id: Field,
    /// 다중 값: 태그 id (태그 필터용)
    pub tag_id: Field,
    /// 수정일 UTC ms — 정렬용 (P2-5)
    pub modified_at: Field,
    /// 이름 정렬 키(파일명 앞 16바이트, 상위/하위 8바이트) — 정렬용 (P2-5)
    pub name_hi: Field,
    pub name_lo: Field,
    /// 제목순 정렬 키 — 제목이 있으면 제목, 없으면 파일명의 앞 16바이트 (상위/하위 8바이트)
    pub title_hi: Field,
    pub title_lo: Field,
    /// 다중 값: `<folder_id>:<조상 디렉터리 path_key>` — 재귀 폴더 범위 (P2-5)
    pub dir_anc: Field,
    /// 단일 값: `<folder_id>:<디렉터리 path_key>` — 비재귀 폴더 범위 (P2-5)
    pub dir_exact: Field,
    /// 사용자 정의 필드 값 — JSON(raw 토크나이저, INDEXED|FAST), 키 `f{field_id}`. FAST 가 없으면 number·date 범위 질의가 거부된다 (Tantivy 0.27)
    pub umeta: Field,
    /// 값이 있는 사용자 필드마다 `f{field_id}` term — "값 있음 / 값 없음" 필터
    pub umeta_has: Field,
    /// 사용자 필드 중 keyword 값 — 일반 키워드 검색용 (ko 분석기)
    pub umeta_text: Field,
}

fn text(tokenizer: &str, option: IndexRecordOption, fieldnorms: bool) -> TextOptions {
    TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(tokenizer)
            .set_index_option(option)
            .set_fieldnorms(fieldnorms),
    )
}

fn umeta_options() -> JsonObjectOptions {
    JsonObjectOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer("raw")
                .set_index_option(IndexRecordOption::Basic)
                .set_fieldnorms(false),
        )
        .set_fast("raw")
}

pub fn build_schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let ko = || text(KOREAN, IndexRecordOption::WithFreqs, true);
    let fields = Fields {
        asset_id: b.add_u64_field("asset_id", INDEXED | STORED | FAST),
        umeta: b.add_json_field("umeta", umeta_options()),
        umeta_has: b.add_text_field("umeta_has", STRING),
        umeta_text: b.add_text_field("umeta_text", ko()),
        modified_at: b.add_i64_field("modified_at", FAST),
        name_hi: b.add_u64_field("name_hi", FAST),
        name_lo: b.add_u64_field("name_lo", FAST),
        title_hi: b.add_u64_field("title_hi", FAST),
        title_lo: b.add_u64_field("title_lo", FAST),
        dir_anc: b.add_text_field("dir_anc", STRING),
        dir_exact: b.add_text_field("dir_exact", STRING),
        revision: b.add_u64_field("revision", STORED | FAST),
        file_name: b.add_text_field(
            "file_name",
            text(FILENAME, IndexRecordOption::WithFreqs, true),
        ),
        file_name_ngram: b.add_text_field(
            "file_name_ngram",
            text(FILENAME_NGRAM, IndexRecordOption::Basic, false),
        ),
        file_name_raw: b.add_text_field(
            "file_name_raw",
            text(FILENAME_RAW, IndexRecordOption::Basic, false),
        ),
        ext: b.add_text_field("ext", STRING),
        user_tags: b.add_text_field("user_tags", ko()),
        ai_keywords: b.add_text_field("ai_keywords", ko()),
        ai_description: b.add_text_field("ai_description", ko()),
        user_memo: b.add_text_field("user_memo", ko()),
        title: b.add_text_field("title", ko()),
        transcript: b.add_text_field("transcript", ko()),
        media_type: b.add_text_field("media_type", STRING),
        volume_id: b.add_u64_field("volume_id", INDEXED | FAST),
        presence: b.add_text_field("presence", STRING),
        captured_at: b.add_i64_field("captured_at", INDEXED | FAST),
        captured_or_modified: b.add_i64_field("captured_or_modified", INDEXED | FAST),
        file_size: b.add_u64_field("file_size", INDEXED | FAST),
        duration_ms: b.add_u64_field("duration_ms", INDEXED | FAST),
        favorite: b.add_u64_field("favorite", INDEXED | FAST),
        rating: b.add_u64_field("rating", INDEXED | FAST),
        collection_id: b.add_u64_field("collection_id", INDEXED | FAST),
        tag_id: b.add_u64_field("tag_id", INDEXED | FAST),
    };
    (b.build(), fields)
}
