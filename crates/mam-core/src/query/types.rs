//! P2-5 계약 타입 (프론트 `ipc-contract.ts` 의 `SearchRequest`·`SearchFilters`·`AssetSummary`·`AssetDetail` 등).
//! 모두 serde camelCase. 필터는 최근/저장 검색에 JSON 으로 저장되므로 필드 순서가 정규 형태다.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::user_field_filter::UserFieldFilter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Image,
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortField {
    Relevance,
    CapturedAt,
    ModifiedAt,
    Name,
    Size,
    Duration,
    Rating,
    /// 컬렉션 안 순서 — `filters.collection_id` 가 있을 때만 유효
    CollectionOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortSpec {
    pub field: SortField,
    pub direction: SortDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderScope {
    pub folder_id: i64,
    /// `None` = 관리 폴더 루트
    pub rel_path: Option<String>,
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilters {
    pub media_types: Vec<MediaKind>,
    pub include_offline: bool,
    pub include_missing: bool,
    /// UTC Unix ms
    pub captured_from: Option<i64>,
    pub captured_to: Option<i64>,
    pub size_min: Option<i64>,
    pub size_max: Option<i64>,
    pub duration_min_ms: Option<i64>,
    pub duration_max_ms: Option<i64>,
    pub folder: Option<FolderScope>,
    pub favorites_only: bool,
    pub collection_id: Option<i64>,
    pub tag_ids: Vec<i64>,
    /// 별점 범위 (포함, 0 = 미평가). 저장한 검색 JSON 에 없으면 `None`
    #[serde(default)]
    pub rating_min: Option<u8>,
    #[serde(default)]
    pub rating_max: Option<u8>,
    /// 사용자 정의 필드 조건 (AND). 저장한 검색 JSON 에 없으면 빈 목록, 비어 있으면 직렬화하지 않아 기존 기록과 같은 JSON 이다
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub user_fields: Vec<UserFieldFilter>,
}

impl SearchFilters {
    /// 아무 조건도 걸지 않은 상태인가 (PL5 기본: 오프라인 표시, 파일 없음 숨김)
    pub fn is_default(&self) -> bool {
        *self
            == Self {
                media_types: vec![],
                include_offline: true,
                include_missing: false,
                captured_from: None,
                captured_to: None,
                size_min: None,
                size_max: None,
                duration_min_ms: None,
                duration_max_ms: None,
                folder: None,
                favorites_only: false,
                collection_id: None,
                tag_ids: vec![],
                rating_min: None,
                rating_max: None,
                user_fields: vec![],
            }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    Keyword,
    Hybrid,
}

/// `search_assets` 인자
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub filters: SearchFilters,
    pub sort: SortSpec,
    pub limit: i64,
    pub cursor: Option<String>,
    #[serde(default)]
    pub mode: Option<SearchMode>,
    /// 생략 = forward. `backward` 는 `cursor` 가 있어야 한다
    #[serde(default)]
    pub direction: Option<PageDirection>,
    /// 전역 0-기준 위치로 바로 가기. `cursor` 와 동시 지정 불가, `>= total` 이면 빈 페이지
    #[serde(default)]
    pub at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageDirection {
    Forward,
    Backward,
}

/// `search_index` 인자 — `search_assets` 와 같은 질의·필터·정렬 (limit·cursor 없음)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexRequest {
    pub query: String,
    pub filters: SearchFilters,
    pub sort: SortSpec,
    #[serde(default)]
    pub mode: Option<SearchMode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexKind {
    Month,
    Initial,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSection {
    /// month: `"YYYY-MM"` / `"none"`(날짜 없음), initial: `"sym"`, `"a"`..`"z"`, `"etc"`, `"ㄱ"`..`"ㅎ"`, `"etc_end"`, none: `"all"`
    pub key: String,
    /// 표시 문구는 프론트가 번역 키/포맷으로 만든다 — 백엔드는 항상 `null`
    pub label: Option<String>,
    pub count: i64,
}

/// `search_index` 반환 — 정렬 순서대로 구간별 개수 (Σcount = total)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndex {
    pub total: i64,
    pub generation: i64,
    pub kind: IndexKind,
    pub sections: Vec<IndexSection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Availability {
    Online,
    Offline,
    Missing,
}

/// 카드 AI 표시용 집계 — none: 결과 없음 / done: 결과 있음 / failed: 결과 없고 (embedding 외) 실패 단계 있음
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AiCardState {
    None,
    Done,
    Failed,
}

/// 갤러리·목록 한 칸 (계약 `AssetSummary`)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetSummary {
    pub asset_id: i64,
    pub file_name: String,
    /// 사용자가 지정한 제목 (없으면 `None`). 설정과 무관한 저장값 그대로 — 표시 여부는 프론트가 `titleEditEnabled` 로 정한다
    pub title: Option<String>,
    pub media_type: MediaKind,
    pub availability: Availability,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub file_size: i64,
    pub captured_at: Option<i64>,
    pub modified_at: i64,
    pub favorite: bool,
    /// 별점 0..=5 (0 = 미평가)
    pub rating: u8,
    /// pending | done | failed | unsupported
    pub thumb_state: String,
    pub revision: i64,
    pub ai_state: AiCardState,
}

/// `search_assets` 반환 (계약 `SearchPage`)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchPage {
    pub items: Vec<AssetSummary>,
    pub total: i64,
    pub next_cursor: Option<String>,
    /// 이 페이지 앞에 항목이 있으면 첫 항목 기준 커서 (`direction: backward` 로 쓴다). 맨 앞·빈 페이지는 `None`
    pub prev_cursor: Option<String>,
    /// `items[0]` 의 전역 0-기준 위치
    pub start_offset: i64,
    /// 색인 세대 (JS 안전 정수로 줄인 값) — `search_index` 와 같은 값이어야 한다
    pub generation: i64,
    pub total_is_estimate: bool,
    pub took_ms: i64,
    /// hybrid 요청을 keyword 로 처리했다
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagRef {
    pub tag_id: i64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectionRef {
    pub collection_id: i64,
    pub name: String,
}

/// `get_asset_detail` 반환 (계약 `AssetDetail`)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetDetail {
    #[serde(flatten)]
    pub summary: AssetSummary,
    /// active | superseded | deleted
    pub status: String,
    pub path: String,
    pub folder_id: Option<i64>,
    pub volume_label: Option<String>,
    pub codec: Option<String>,
    pub audio_codec: Option<String>,
    pub fps: Option<f64>,
    pub bitrate: Option<i64>,
    pub camera: Option<String>,
    pub extra: BTreeMap<String, String>,
    pub playable: bool,
    /// pending | done | failed
    pub hash_state: String,
    pub tags: Vec<TagRef>,
    pub memo: String,
    pub collections: Vec<CollectionRef>,
    pub duplicates: Vec<AssetSummary>,
    pub previous_versions: Vec<AssetSummary>,
    pub newer_version: Option<AssetSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentSearch {
    pub query: String,
    pub filters: SearchFilters,
    pub used_at: i64,
}
