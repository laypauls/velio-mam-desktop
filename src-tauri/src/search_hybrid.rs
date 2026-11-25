//! 하이브리드 검색(`mode = "hybrid"`)과 유사 미디어(`find_similar`) — P3 (P3-6 결합 코어 + P3-5 임베딩 저장 연결).
//!
//! 흐름 (하이브리드):
//! 1. 필터를 통과한 자산 집합을 Tantivy 에서 먼저 구한다 -> 그 자산의 활성 `embedding_id` 만 벡터 후보 대상 (순위 전 필터).
//! 2. 키워드 상위 N(BM25, 필터 적용) + 벡터 상위 N(코사인) -> `fuse_hybrid`(RRF). 점수를 더하지 않고 순위만 쓴다.
//! 3. 후보는 페이지 끝의 4배(최소 400, 최대 2,000). 결과가 모자라고 후보가 꽉 찼으면 두 배씩 늘린다.
//!
//! 질의 임베딩 모델·원시 임베딩 저장소가 주입되지 않았거나(모델 미설치), 활성 인덱스가 없거나, 벡터 쪽이 실패하면
//! `None` 을 돌려주고 호출자는 키워드 결과 + `degraded` 로 응답한다 (검색 자체는 실패시키지 않는다).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, RwLock};

use mam_core::query::{AssetSummary, SearchFilters};
use mam_core::scan::IngestError;
use mam_search::coordinator::{CoordRequest, Direction, MAX_PAGE_LIMIT};
use mam_search::{Filters, SearchError};
use mam_vector::{
    ActiveMap, ActiveMapCache, AnnIndex, EmbedStore, EmbeddingId, Generations, IdSet, ModelKey,
    OpenMode, RrfParams, SimilarCursor, SimilarQuery, VectorError, find_similar, fuse_hybrid,
    needs_more, plan_candidates, vector_candidates_with_set,
};
use serde::{Deserialize, Serialize};

use crate::allowed_cache::AllowedCache;
use crate::search_service::{SearchFailure, SearchService};

/// 질의 벡터를 만드는 주입 경계. 실제 구현(e5 `query:` 임베딩)은 AI 어댑터가 제공한다.
///
/// 모델이 설치되지 않았으면 `semantic_key()` 가 `None` 이다 -> 하이브리드는 키워드 + `degraded`.
pub trait QueryEmbedder: Send + Sync {
    /// 의미(e5) 벡터의 모델 키 — 저장된 벡터의 모델 ID·버전·차원과 같아야 한다 (모델 미설치면 `None`).
    fn semantic_key(&self) -> Option<ModelKey>;
    /// 시각(CLIP) 벡터의 모델 키 — `find_similar` 가 활성 인덱스를 고르는 데 쓴다. 질의 인코더는 필요 없다.
    fn visual_key(&self) -> Option<ModelKey>;
    /// 질의 문장 -> 정규화된 의미 벡터 (`mam_core::embedding::embed_query(.., Semantic, text)`). 실패는 `Err`(사유 문자열).
    fn embed_semantic_query(&self, text: &str) -> Result<Vec<f32>, String>;
}

/// 주입 슬롯 — 앱 시작 후 AI 어댑터가 채운다 (둘 다 있어야 벡터 검색이 켜진다).
#[derive(Default)]
pub(crate) struct VectorSlots {
    embedder: RwLock<Option<Arc<dyn QueryEmbedder>>>,
    store: RwLock<Option<Arc<EmbedStore>>>,
    /// 모델 키별 `embedding_id -> asset_id` 활성 맵 캐시. 100만 벡터에서 SQLite 읽기가 질의당 약 1초라 재사용한다
    /// (P4 종단 측정: 하이브리드 질의 약 2초 중 절반). `put`·세대 교체는 자동 감지, 그 밖의 변경은 `invalidate_all`.
    cache: ActiveMapCache,
    /// 필터를 통과한 자산 집합의 짧은 캐시 (`allowed_cache`)
    pub(crate) allowed: AllowedCache,
    /// 시험·측정 전용: 켜면 허용 집합을 캐시·빠른 수집 없이 예전 방식(`DocSetCollector` 전체 수집)으로 만든다
    #[cfg(test)]
    pub(crate) legacy_allowed: std::sync::atomic::AtomicBool,
}

impl VectorSlots {
    pub(crate) fn set_embedder(&self, e: Arc<dyn QueryEmbedder>) {
        *self.embedder.write().unwrap_or_else(|p| p.into_inner()) = Some(e);
    }

    pub(crate) fn set_store(&self, s: Arc<EmbedStore>) {
        *self.store.write().unwrap_or_else(|p| p.into_inner()) = Some(s);
    }

    fn parts(&self) -> Option<(Arc<dyn QueryEmbedder>, Arc<EmbedStore>)> {
        let e = self.embedder.read().unwrap_or_else(|p| p.into_inner());
        let s = self.store.read().unwrap_or_else(|p| p.into_inner());
        Some((Arc::clone(e.as_ref()?), Arc::clone(s.as_ref()?)))
    }
}

/// 하이브리드 한 페이지 (id 는 RRF 순서).
pub(crate) struct HybridPage {
    pub ids: Vec<u64>,
    /// 결합된 후보 수 (전체 일치 수가 아니다 -> `totalIsEstimate`)
    pub total: usize,
    pub next_cursor: Option<String>,
    pub prev_cursor: Option<String>,
    /// `ids[0]` 의 위치 (범위 밖 `at` 은 후보 수로 줄인다)
    pub start_offset: usize,
    pub generation: u64,
}

/// `find_similar` 요청 (계약 `SimilarRequest`)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarRequest {
    pub asset_id: i64,
    pub limit: i64,
    pub cursor: Option<String>,
}

/// 계약 `SimilarItem` = `AssetSummary` + `similarity`
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarItem {
    #[serde(flatten)]
    pub summary: AssetSummary,
    pub similarity: f32,
}

/// 계약 `Page<SimilarItem>` (`total` = 이번 페이지 항목 수 — 전체 수는 계산하지 않는다)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimilarPage {
    pub items: Vec<SimilarItem>,
    pub total: i64,
    pub next_cursor: Option<String>,
}

fn vector_failure(what: &'static str, e: &VectorError) {
    tracing::warn!(error = %e, "{what}; falling back");
}

/// 하이브리드 커서: 인덱스 세대 + 요청 지문(질의·필터) + 시작 위치.
fn fingerprint(text: &str, filters: &Filters) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    format!("{filters:?}").hash(&mut h);
    h.finish()
}

fn encode_cursor(generation: u64, print: u64, offset: usize) -> String {
    format!("h1.{generation:x}.{print:x}.{offset:x}")
}

fn decode_cursor(c: &str, generation: u64, print: u64) -> Result<usize, SearchError> {
    let bad = || SearchError::InvalidInput("malformed hybrid cursor".into());
    let mut it = c.split('.');
    let (Some("h1"), Some(g), Some(p), Some(o), None) =
        (it.next(), it.next(), it.next(), it.next(), it.next())
    else {
        return Err(bad());
    };
    let parse = |s: &str| u64::from_str_radix(s, 16).map_err(|_| bad());
    let (g, p, o) = (parse(g)?, parse(p)?, parse(o)?);
    if g != generation || p != print {
        return Err(SearchError::StaleCursor);
    }
    usize::try_from(o).map_err(|_| bad())
}

fn invalid(what: &'static str) -> SearchFailure {
    SearchFailure::Ingest(IngestError::InvalidInput(what))
}

impl SearchService {
    /// 질의 임베딩 모델(주입 경계)을 연결한다. 모델이 바뀌면(설치·교체) 같은 객체가 새 키를 돌려주면 된다.
    pub fn set_query_embedder(&self, embedder: Arc<dyn QueryEmbedder>) {
        self.vectors.set_embedder(embedder);
    }

    /// 원시 임베딩 저장소를 연결한다 (`mam_vector::EmbedStore` — 임베딩 작업이 쓰는 것과 같은 루트).
    pub fn set_embed_store(&self, store: Arc<EmbedStore>) {
        self.vectors.set_store(store);
    }

    /// 하이브리드 한 페이지. 벡터 검색을 쓸 수 없으면 `Ok(None)` (호출자가 키워드 + `degraded`).
    pub(crate) fn hybrid_page(
        &self,
        coord: &CoordRequest,
    ) -> Result<Option<HybridPage>, SearchFailure> {
        let Some((embedder, store)) = self.vectors.parts() else {
            return Ok(None);
        };
        let Some(key) = embedder.semantic_key() else {
            return Ok(None);
        };
        let generation = self.handle.generation();
        let print = fingerprint(&coord.text, &coord.filters);
        // 하이브리드 커서 = 경계 위치. forward 는 그 위치부터, backward 는 그 위치 앞의 limit 건 (`at` 는 위치 직접 지정)
        if coord.cursor.is_some() && coord.at.is_some() {
            return Err(SearchFailure::Search(SearchError::InvalidInput(
                "cursor and at are mutually exclusive".into(),
            )));
        }
        let backward = coord.direction == Direction::Backward;
        if backward && coord.cursor.is_none() {
            return Err(SearchFailure::Search(SearchError::InvalidInput(
                "backward paging requires a cursor".into(),
            )));
        }
        let boundary = match &coord.cursor {
            Some(c) => Some(decode_cursor(c, generation, print)?),
            None => None,
        };
        let (offset, take) = match (boundary, coord.at) {
            (Some(b), _) if backward => {
                let start = b.saturating_sub(coord.limit);
                (start, b - start)
            }
            (Some(b), _) => (b, coord.limit),
            (None, at) => (at.unwrap_or(0), coord.limit),
        };
        let query = match embedder.embed_semantic_query(&coord.text) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "cannot embed the search query; falling back");
                return Ok(None);
            }
        };
        let Some(index) = open_index(&store, &key)? else {
            return Ok(None);
        };
        let map = self.active_map(&store, &key)?;

        // 순위 전 필터: 허용 자산(짧은 캐시) -> 그 자산의 활성 embedding_id. 집합은 질의당 한 번만 만든다
        let (allowed, allowed_set) = self.allowed_embeddings(&map, &coord.filters)?;

        let page_end = offset.saturating_add(coord.limit);
        let mut plan = plan_candidates(page_end.saturating_add(1));
        loop {
            let keyword = self.handle.keyword_candidates(
                &coord.text,
                coord.strategy,
                &coord.filters,
                plan.per_source,
            )?;
            let vector = match vector_candidates_with_set(
                &index,
                &query,
                &plan,
                Some(&allowed),
                Some(&allowed_set),
                index.len(),
                &|id| map.asset_of(id),
            ) {
                Ok(v) => v,
                Err(e) => {
                    vector_failure("vector candidates failed", &e);
                    return Ok(None);
                }
            };
            let fused = fuse_hybrid(&keyword, Some(&vector), &RrfParams::default()).hits;
            if needs_more(
                fused.len(),
                page_end + 1,
                keyword.len(),
                vector.len(),
                &plan,
            ) {
                plan = plan.grow();
                continue;
            }
            let ids: Vec<u64> = fused
                .iter()
                .skip(offset)
                .take(take)
                .map(|h| h.asset_id)
                .collect();
            let end = offset.saturating_add(ids.len());
            let start = offset.min(fused.len());
            return Ok(Some(HybridPage {
                total: fused.len(),
                next_cursor: (!ids.is_empty() && fused.len() > end)
                    .then(|| encode_cursor(generation, print, end)),
                prev_cursor: (!ids.is_empty() && start > 0)
                    .then(|| encode_cursor(generation, print, start)),
                start_offset: start,
                generation,
                ids,
            }));
        }
    }

    /// 필터를 통과한 활성 임베딩 id 와 그 판정용 집합.
    ///
    /// 허용 자산은 색인에 있고 필터를 통과한 자산뿐이다 — 활성 맵에는 'superseded'·'deleted' 자산의 벡터도 남아 있어
    /// (CASCADE 는 영구 삭제에만 걸린다) "활성 전체"로 대신하면 낡은 버전·오프라인·파일 없음 자산이 새어 나온다.
    fn allowed_embeddings(
        &self,
        map: &ActiveMap,
        filters: &Filters,
    ) -> Result<(Vec<EmbeddingId>, IdSet), SearchFailure> {
        #[cfg(test)]
        if self
            .vectors
            .legacy_allowed
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let assets = self.handle.filtered_asset_ids_docset(filters)?;
            let allowed = map.embeddings_of_assets(&assets);
            let set = IdSet::from_ids(&allowed);
            return Ok((allowed, set));
        }
        let assets = self.vectors.allowed.assets(&self.handle, filters)?;
        let allowed = map.embeddings_in(&assets);
        let set = IdSet::from_ids(&allowed);
        Ok((allowed, set))
    }

    /// 활성 `embedding_id -> asset_id` (폐기된 벡터 제외). `ActiveMapCache` 가 `put`·세대 교체를 스스로 감지한다.
    fn active_map(
        &self,
        store: &EmbedStore,
        key: &ModelKey,
    ) -> Result<Arc<ActiveMap>, SearchFailure> {
        let read = self
            .pool
            .read(|c| Ok(self.vectors.cache.get_or_load(c, store, key)))
            .map_err(IngestError::from)?;
        read.map_err(|e| {
            tracing::warn!(error = %e, "cannot read the embedding map");
            SearchFailure::Ingest(IngestError::InvalidInput("embeddings"))
        })
    }

    /// 의미·시각 키의 활성 맵을 미리 읽어 첫 질의가 전체 로드(100만 벡터 약 1초)를 겪지 않게 한다.
    /// 호출자는 백그라운드 스레드에서 부른다 — 저장소·모델이 아직 없으면 조용히 건너뛴다. 로드한 맵 수를 돌려준다.
    pub fn warm_vector_maps(&self) -> usize {
        let Some((embedder, store)) = self.vectors.parts() else {
            return 0;
        };
        let mut warmed = 0;
        for key in [embedder.semantic_key(), embedder.visual_key()]
            .into_iter()
            .flatten()
        {
            if self.active_map(&store, &key).is_ok() {
                warmed += 1;
            }
        }
        warmed
    }

    /// UI 기본 필터(오프라인 포함, 파일 없음 제외)의 허용 자산 집합을 미리 만든다 - 첫 하이브리드·유사 질의가 100만 건 수집
    /// (수백 ms)을 겪지 않게 한다. 색인이 열릴 때까지 `max_wait` 안에서 기다린다. 열리지 않았으면 `false`.
    pub fn warm_allowed_set(&self, max_wait: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + max_wait;
        while !self.handle.is_ready() {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        let defaults = SearchFilters {
            media_types: Vec::new(),
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
            tag_ids: Vec::new(),
            rating_min: None,
            rating_max: None,
            user_fields: vec![],
        };
        let Ok(filters) = self.filters(&defaults) else {
            return false;
        };
        self.vectors.allowed.assets(&self.handle, &filters).is_ok()
    }

    /// 유사 미디어 한 페이지 (시각 벡터 활성 인덱스). 기준 자산 자신과 같은 내용 해시의 중복본은 제외한다.
    /// 모델·저장소·인덱스가 없거나 기준 자산에 시각 벡터가 없으면 빈 페이지다.
    pub fn find_similar(&self, req: &SimilarRequest) -> Result<SimilarPage, SearchFailure> {
        let asset_id = u64::try_from(req.asset_id)
            .ok()
            .filter(|a| *a > 0)
            .ok_or_else(|| invalid("assetId"))?;
        let limit = usize::try_from(req.limit)
            .ok()
            .filter(|l| (1..=MAX_PAGE_LIMIT).contains(l))
            .ok_or_else(|| invalid("limit"))?;
        let cursor = match &req.cursor {
            Some(c) => Some(SimilarCursor::decode(c).map_err(|_| {
                SearchFailure::Search(SearchError::InvalidInput("malformed similar cursor".into()))
            })?),
            None => None,
        };
        let empty = SimilarPage {
            items: Vec::new(),
            total: 0,
            next_cursor: None,
        };

        let exclude = self.same_content_assets(asset_id)?; // 존재하지 않는 자산이면 NotFound
        let Some((embedder, store)) = self.vectors.parts() else {
            return Ok(empty);
        };
        let Some(key) = embedder.visual_key() else {
            return Ok(empty);
        };
        let frames = match self
            .pool
            .read(|c| Ok(store.asset_vectors(c, &key, asset_id)))
            .map_err(IngestError::from)?
        {
            Ok(f) if !f.is_empty() => f,
            Ok(_) => return Ok(empty),
            Err(e) => {
                vector_failure("cannot read the base vectors", &e);
                return Ok(empty);
            }
        };
        let Some(index) = open_index(&store, &key)? else {
            return Ok(empty);
        };
        let map = self.active_map(&store, &key)?;
        // 색인된 자산(활성 + 활성 위치)만 후보 — 오프라인 볼륨의 자산도 포함한다
        let hidden_folder_ids = self.query.hidden_folder_ids()?;
        let visible = self.vectors.allowed.assets(
            &self.handle,
            &Filters {
                hidden_folder_ids,
                ..Filters::default()
            },
        )?;
        let asset_of = |id: EmbeddingId| map.asset_of(id).filter(|a| visible.contains(*a));

        let found = match find_similar(
            &index,
            &SimilarQuery {
                frames: &frames,
                exclude: &exclude,
                limit,
                cursor,
                min_score: self.similar_min_score(),
            },
            &asset_of,
        ) {
            Ok(p) => p,
            Err(e) => {
                vector_failure("similar search failed", &e);
                return Ok(empty);
            }
        };

        let ids: Vec<i64> = found.items.iter().map(|(a, _)| *a as i64).collect();
        let similarity: HashMap<i64, f32> =
            found.items.iter().map(|(a, s)| (*a as i64, *s)).collect();
        let items: Vec<SimilarItem> = self
            .query
            .hydrate(&ids)?
            .into_iter()
            .filter_map(|summary| {
                let similarity = *similarity.get(&summary.asset_id)?;
                Some(SimilarItem {
                    summary,
                    similarity,
                })
            })
            .collect();
        Ok(SimilarPage {
            total: i64::try_from(items.len()).unwrap_or(i64::MAX),
            items,
            next_cursor: found.next.map(|c| c.encode()),
        })
    }

    /// 설정 `similarMinSimilarity` (매 호출 읽음). 읽지 못하면 기본값(0.75).
    fn similar_min_score(&self) -> f32 {
        let v = self
            .pool
            .read(|c| mam_db::settings::load(c).map(|(s, _)| s.similar_min_similarity))
            .unwrap_or_else(|_| mam_db::Settings::default().similar_min_similarity);
        (v as f32).clamp(0.0, 1.0)
    }

    /// 기준 자산 자신 + 같은 전체 내용 해시의 다른 자산 (`idx_media_assets_hash`). 활성 자산이 아니면 `NotFound`.
    fn same_content_assets(&self, asset_id: u64) -> Result<Vec<u64>, SearchFailure> {
        let found = self
            .pool
            .read(|c| Ok(mam_core::assets::same_content(c, asset_id as i64)))
            .map_err(IngestError::from)?;
        match found {
            Ok(Some(ids)) => Ok(ids.into_iter().map(|id| id as u64).collect()),
            Ok(None) => Err(SearchFailure::Ingest(IngestError::NotFound)),
            Err(e) => {
                tracing::warn!(error = %e, "cannot read the duplicate assets");
                Err(SearchFailure::Ingest(IngestError::InvalidInput("assetId")))
            }
        }
    }
}

/// 활성 세대를 읽기 전용 매핑으로 연다. 없거나 열 수 없으면 `None` (재구축은 임베딩 쪽 책임이다).
fn open_index(
    store: &EmbedStore,
    key: &ModelKey,
) -> Result<Option<mam_vector::FlatIndex>, SearchFailure> {
    let opened = Generations::new(store, key).and_then(|g| g.open_active(key, OpenMode::View));
    match opened {
        Ok(idx) => Ok(idx),
        Err(e) => {
            vector_failure("cannot open the active vector index", &e);
            Ok(None)
        }
    }
}

#[cfg(test)]
#[path = "search_hybrid_tests.rs"]
mod tests;
