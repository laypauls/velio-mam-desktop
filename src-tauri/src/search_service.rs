//! 검색 서비스 (P2-5) — 색인 워커 스레드 + 검색 조정(필터 변환 -> Tantivy -> SQLite 하이드레이션).
//!
//! - 시작 시 백그라운드 스레드가 `SyncWorker` 를 연다(필요하면 SQLite 에서 재구축). 재구축 중에는 검색이
//!   `NotReady` 로 실패하고, 끝나면 같은 핸들로 검색된다 (`SearchHandle` 은 워커와 독립).
//! - 이후 Outbox 를 배치 단위로 처리한다. 배치마다 쓰기 큐에 짧은 작업으로 넣어 사용자 편집을 막지 않고,
//!   사용자 편집 command 가 `wake()` 로 깨우면 priority 0 이벤트를 즉시 처리한다 (D9).
//! - 하이브리드(`mode = hybrid`, 관련도 정렬)는 `search_hybrid` 가 키워드·벡터 후보를 RRF 로 결합한다. 질의 임베딩 모델·
//!   임베딩 저장소가 주입되지 않았거나 활성 벡터 인덱스가 없으면 키워드 결과를 `degraded` 로 돌려준다.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::query::{
    FolderScope, IndexKind, IndexSection, MediaKind, PageDirection, QueryService, SearchFilters,
    SearchIndex, SearchIndexRequest, SearchMode, SearchPage, SearchRequest, SortDirection,
    SortField as ApiSortField,
};
use mam_core::scan::IngestError;
use mam_db::{DbPool, PoolError, Priority};
use mam_search::coordinator::{
    CoordPage, CoordRequest, Direction, MAX_PAGE_LIMIT, SearchHandle, SectionKind, SortField,
    SortSpec, local_utc_offset_secs,
};
use mam_search::search::FolderFilter;
use mam_search::sync::{SyncConfig, SyncError, SyncWorker, write_progress_on};
use mam_search::{
    FilenameStrategy, Filters, IndexOptions, MediaType, Presence, Range, SearchError,
};

use crate::error::{CommandError, ErrorCode};
use crate::search_hybrid::VectorSlots;

/// 유휴 시 Outbox 를 확인하는 간격
const IDLE_POLL: Duration = Duration::from_secs(1);
/// 실패·중단 뒤 재시도 간격
const ERROR_BACKOFF: Duration = Duration::from_secs(5);
/// 태그 필터 최대 개수
const MAX_TAG_FILTERS: usize = 50;

/// 검색 실패 — command 경계에서 `CommandError` 로 바뀐다.
#[derive(Debug)]
pub enum SearchFailure {
    Ingest(IngestError),
    Search(SearchError),
}

impl From<IngestError> for SearchFailure {
    fn from(e: IngestError) -> Self {
        Self::Ingest(e)
    }
}

impl From<SearchError> for SearchFailure {
    fn from(e: SearchError) -> Self {
        Self::Search(e)
    }
}

impl From<SearchFailure> for CommandError {
    fn from(e: SearchFailure) -> Self {
        match e {
            SearchFailure::Ingest(e) => e.into(),
            SearchFailure::Search(SearchError::InvalidInput(m)) => {
                Self::new(ErrorCode::InvalidInput, format!("invalid input: {m}"))
            }
            // 재구축 등으로 커서가 무효가 됐다 — UI 는 첫 페이지부터 다시 요청한다
            SearchFailure::Search(SearchError::StaleCursor) => {
                Self::new(ErrorCode::InvalidInput, "invalid input: stale cursor")
            }
            SearchFailure::Search(SearchError::NotReady) => {
                Self::new(ErrorCode::IndexNotReady, "search index is not ready")
            }
            SearchFailure::Search(other) => {
                Self::logged(ErrorCode::Internal, "search operation failed", &other)
            }
        }
    }
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// 색인 세대는 시작 시각을 상위 비트에 담은 u64 라 JSON 숫자(JS number)로 보내면 2^53 위가 잘려 연속한 세대가 같아 보인다.
/// 하위 53비트만 보낸다 — 세대 증가(+1)는 하위 비트에서 일어나므로 같은 프로세스 안에서 구분이 유지된다.
pub(crate) fn js_safe_generation(g: u64) -> i64 {
    (g & ((1u64 << 53) - 1)) as i64
}

#[derive(Default)]
struct Wake {
    flag: Mutex<bool>,
    cond: Condvar,
}

impl Wake {
    fn notify(&self) {
        *self.flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.cond.notify_all();
    }

    /// 깨우기 신호나 `timeout` 까지 기다린다.
    fn wait(&self, timeout: Duration) {
        let mut flag = self.flag.lock().unwrap_or_else(|e| e.into_inner());
        if !*flag {
            flag = self
                .cond
                .wait_timeout(flag, timeout)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *flag = false;
    }
}

pub struct SearchService {
    pub(crate) handle: SearchHandle,
    pub(crate) query: Arc<QueryService>,
    pub(crate) pool: Arc<DbPool>,
    /// 질의 임베딩·원시 임베딩 저장소 주입 슬롯 (하이브리드·유사 미디어, `search_hybrid`)
    pub(crate) vectors: VectorSlots,
    wake: Arc<Wake>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl SearchService {
    /// 서비스를 만들고 색인 워커 스레드를 시작한다. 인덱스 디렉터리는 `MAMData/indexes/tantivy`.
    pub fn start(
        pool: Arc<DbPool>,
        clock: SharedClock,
        index_dir: PathBuf,
        query: Arc<QueryService>,
    ) -> Arc<Self> {
        let handle = SearchHandle::new();
        let wake = Arc::new(Wake::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (handle, wake, stop) = (handle.clone(), Arc::clone(&wake), Arc::clone(&stop));
            let pool = Arc::clone(&pool);
            std::thread::Builder::new()
                .name("mam-search-sync".into())
                .spawn(move || run_worker(&pool, &clock, &index_dir, handle, &wake, &stop))
                .map_err(|e| tracing::error!(%e, "cannot start search sync thread"))
                .ok()
        };
        Arc::new(Self {
            handle,
            query,
            pool,
            vectors: VectorSlots::default(),
            wake,
            stop,
            thread: Mutex::new(thread),
        })
    }

    /// 사용자 편집 직후 호출 — 워커가 priority 0 이벤트를 바로 처리하게 한다.
    pub fn wake(&self) {
        self.wake.notify();
    }

    /// 워커 스레드를 멈추고 기다린다 (DB 풀 종료보다 먼저).
    #[cfg(test)]
    pub fn shutdown(&self) {
        self.shutdown_within(Duration::MAX);
    }

    /// 종료 신호만 보낸다 (기다리지 않는다).
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.wake.notify();
    }

    /// [`Self::shutdown`] 이되 `grace` 를 넘기면 기다리지 않는다 (큰 배치 커밋·재구축 중일 수 있다). 떼어 낸 스레드 수를 돌려준다.
    /// 색인은 파생 데이터라 중간에 끊겨도 `search_outbox` 와 시작 시 점검으로 다시 맞춘다.
    pub fn shutdown_within(&self, grace: Duration) -> usize {
        self.request_stop();
        let t = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take();
        let abandoned = mam_core::join::join_within(t.into_iter().collect(), grace);
        if abandoned > 0 {
            tracing::warn!("search worker still busy at shutdown; not waiting");
        }
        abandoned
    }

    /// 검색 한 페이지 (blocking — `spawn_blocking` 에서 호출).
    pub fn search(&self, req: &SearchRequest) -> Result<SearchPage, SearchFailure> {
        let started = Instant::now();
        let mut coord = self.coord_request(req)?;
        self.attach_collection_order(&mut coord, req.filters.collection_id)?;
        let hybrid = req.mode == Some(SearchMode::Hybrid) && !coord.text.is_empty();
        // 하이브리드는 관련도 순위 결합이다 — 필드 정렬(촬영일 등)을 명시하면 키워드로 처리한다 (degraded 표시)
        if hybrid
            && coord.sort.field == SortField::Relevance
            && let Some(h) = self.hybrid_page(&coord)?
        {
            let ids: Vec<i64> = h.ids.iter().map(|id| *id as i64).collect();
            return Ok(SearchPage {
                items: self.query.hydrate(&ids)?,
                total: i64::try_from(h.total).unwrap_or(i64::MAX),
                next_cursor: h.next_cursor,
                prev_cursor: h.prev_cursor,
                start_offset: to_i64(h.start_offset as u64),
                generation: js_safe_generation(h.generation),
                // 결합된 후보 수이지 전체 일치 수가 아니다
                total_is_estimate: true,
                took_ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
                degraded: None,
            });
        }
        let page: CoordPage = self.handle.search(&coord)?;
        let ids: Vec<i64> = page.ids.iter().map(|id| *id as i64).collect();
        let items = self.query.hydrate(&ids)?;
        // 모델·인덱스가 없거나 필드 정렬이면 키워드 결과로 응답하고 표시한다
        let degraded = hybrid.then_some(true);
        Ok(SearchPage {
            items,
            total: i64::try_from(page.total).unwrap_or(i64::MAX),
            next_cursor: page.next_cursor,
            prev_cursor: page.prev_cursor,
            start_offset: to_i64(page.start_offset),
            generation: js_safe_generation(page.generation),
            total_is_estimate: false,
            took_ms: i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX),
            degraded,
        })
    }

    fn coord_request(&self, req: &SearchRequest) -> Result<CoordRequest, SearchFailure> {
        let invalid = |what| SearchFailure::Ingest(IngestError::InvalidInput(what));
        let limit = usize::try_from(req.limit)
            .ok()
            .filter(|l| (1..=MAX_PAGE_LIMIT).contains(l))
            .ok_or_else(|| invalid("limit"))?;
        let text = req.query.trim().to_string();
        if text.chars().count() > mam_core::query::MAX_QUERY_CHARS {
            return Err(invalid("query"));
        }
        let title_enabled = self.title_enabled();
        let mut filters = self.filters(&req.filters)?;
        filters.search_title = title_enabled;
        let captured_fallback = filters.captured_fallback;
        Ok(CoordRequest {
            text,
            strategy: FilenameStrategy::B,
            filters,
            sort: SortSpec {
                field: match req.sort.field {
                    ApiSortField::Relevance => SortField::Relevance,
                    ApiSortField::CapturedAt if captured_fallback => SortField::CapturedOrModified,
                    ApiSortField::CapturedAt => SortField::CapturedAt,
                    ApiSortField::ModifiedAt => SortField::ModifiedAt,
                    ApiSortField::Name => self.name_sort_field(title_enabled),
                    ApiSortField::Size => SortField::Size,
                    ApiSortField::Duration => SortField::Duration,
                    ApiSortField::Rating => SortField::Rating,
                    ApiSortField::CollectionOrder if req.filters.collection_id.is_some() => {
                        SortField::CollectionOrder
                    }
                    ApiSortField::CollectionOrder => return Err(invalid("sort")),
                },
                descending: req.sort.direction == SortDirection::Desc,
            },
            limit,
            cursor: req.cursor.clone(),
            direction: match req.direction {
                Some(PageDirection::Backward) => Direction::Backward,
                _ => Direction::Forward,
            },
            at: match req.at {
                Some(a) => Some(usize::try_from(a).map_err(|_| invalid("at"))?),
                None => None,
            },
            collection_order: None,
        })
    }

    /// 목차·행 모델용 구간 분포 (blocking). 월 경계는 OS 로컬 시간대 — 프론트 월 머리글과 같다.
    pub fn search_index(&self, req: &SearchIndexRequest) -> Result<SearchIndex, SearchFailure> {
        let as_search = SearchRequest {
            query: req.query.clone(),
            filters: req.filters.clone(),
            sort: req.sort,
            limit: 1,
            cursor: None,
            mode: req.mode,
            direction: None,
            at: None,
        };
        let coord = self.coord_request(&as_search)?;
        let idx = self.handle.section_index(&coord, &local_utc_offset_secs)?;
        Ok(SearchIndex {
            total: to_i64(idx.total),
            generation: js_safe_generation(idx.generation),
            kind: match idx.kind {
                SectionKind::Month => IndexKind::Month,
                SectionKind::Initial => IndexKind::Initial,
                SectionKind::None => IndexKind::None,
            },
            sections: idx
                .sections
                .into_iter()
                .map(|s| IndexSection {
                    key: s.key,
                    label: None,
                    count: to_i64(s.count),
                })
                .collect(),
        })
    }

    /// 설정 `titleEditEnabled` (읽지 못하면 꺼짐 = 파일명 기준·제목 비검색).
    fn title_enabled(&self) -> bool {
        self.pool
            .read(|c| mam_db::settings::load(c).map(|(s, _)| s.title_edit_enabled))
            .unwrap_or(false)
    }

    /// 설정 `capturedAtFallback` 이 `modified` 인가 (읽지 못하면 기본값 = 수정일 대체 켜짐).
    /// 촬영일 정렬·목차·범위 필터가 `captured_or_modified` 를 쓴다. 두 필드를 모두 색인해 두므로 토글에 재색인이 필요 없다.
    fn captured_fallback_enabled(&self) -> bool {
        self.pool
            .read(|c| mam_db::settings::load(c).map(|(s, _)| s.captured_at_fallback))
            .unwrap_or_default()
            == mam_db::CapturedAtFallback::Modified
    }

    /// API `name` 정렬이 가리킬 인덱스 키: 제목 편집이 켜져 있으면 제목순(제목 없으면 파일명), 꺼져 있으면 파일명.
    /// 두 키를 모두 색인해 두므로 설정을 바꿔도 재색인이 필요 없다. 설정을 못 읽으면 파일명 기준으로 되돌린다.
    fn name_sort_field(&self, title_enabled: bool) -> SortField {
        if title_enabled {
            SortField::Title
        } else {
            SortField::Name
        }
    }

    pub(crate) fn filters(&self, f: &SearchFilters) -> Result<Filters, SearchFailure> {
        let invalid = |what| SearchFailure::Ingest(IngestError::InvalidInput(what));
        let non_negative = |v: Option<i64>, what| match v {
            Some(v) if v < 0 => Err(invalid(what)),
            other => Ok(other.map(|v| v as u64)),
        };
        // 사용자 필드 조건: 현재 정의와 대조(정의 조회 1회, 조건이 없으면 0회) 후 변환
        self.query.validate_user_fields(&f.user_fields)?;
        let user_fields = crate::search_user_fields::to_conditions(&f.user_fields)?;
        if f.tag_ids.len() > MAX_TAG_FILTERS {
            return Err(invalid("tagIds"));
        }
        let mut tag_ids: Vec<u64> = f
            .tag_ids
            .iter()
            .map(|t| u64::try_from(*t).map_err(|_| invalid("tagIds")))
            .collect::<Result<_, _>>()?;
        tag_ids.sort_unstable();
        tag_ids.dedup();
        let folder = match &f.folder {
            Some(scope) => Some(self.folder_filter(scope)?),
            None => None,
        };
        // 숨김 폴더 제외: 폴더 범위·컬렉션을 직접 지정한 보기에서는 적용하지 않는다 (그 안의 항목은 보여 준다)
        let hidden_folder_ids = if f.folder.is_some() || f.collection_id.is_some() {
            Vec::new()
        } else {
            self.query.hidden_folder_ids()?
        };
        // 별점은 0..=5 (0 = 미평가) — 범위 밖은 거부한다
        let rating_bound = |v: Option<u8>, what| match v {
            Some(v) if v > 5 => Err(invalid(what)),
            other => Ok(other.map(u64::from)),
        };
        // 인덱스의 촬영일은 유닉스 초다 (SQLite 는 ms)
        let secs = |ms: Option<i64>| ms.map(|v| v.div_euclid(1000));
        Ok(Filters {
            media_types: f
                .media_types
                .iter()
                .map(|m| match m {
                    MediaKind::Image => MediaType::Image,
                    MediaKind::Video => MediaType::Video,
                    MediaKind::Audio => MediaType::Audio,
                })
                .collect(),
            online_volume_ids: if f.include_offline {
                None
            } else {
                Some(self.query.online_volume_ids()?)
            },
            presence: (!f.include_missing).then_some(Presence::Present),
            captured_at: Range {
                min: secs(f.captured_from),
                max: secs(f.captured_to),
            },
            file_size: Range {
                min: non_negative(f.size_min, "sizeMin")?,
                max: non_negative(f.size_max, "sizeMax")?,
            },
            duration_ms: Range {
                min: non_negative(f.duration_min_ms, "durationMinMs")?,
                max: non_negative(f.duration_max_ms, "durationMaxMs")?,
            },
            folder,
            favorite_only: f.favorites_only,
            rating: Range {
                min: rating_bound(f.rating_min, "ratingMin")?,
                max: rating_bound(f.rating_max, "ratingMax")?,
            },
            collection_id: match f.collection_id {
                Some(c) => Some(u64::try_from(c).map_err(|_| invalid("collectionId"))?),
                None => None,
            },
            tag_ids,
            search_title: false, // coord_request 가 설정으로 채운다
            captured_fallback: self.captured_fallback_enabled(),
            hidden_folder_ids,
            user_fields,
        })
    }

    fn folder_filter(&self, scope: &FolderScope) -> Result<FolderFilter, SearchFailure> {
        let (folder_id, dir_key) = self.query.resolve_folder(scope)?;
        Ok(FolderFilter {
            folder_id,
            dir_key,
            recursive: scope.recursive,
        })
    }
}

/// 워커 스레드 본체: 열기(필요 시 재구축) -> Outbox 처리 루프.
fn run_worker(
    pool: &Arc<DbPool>,
    clock: &SharedClock,
    dir: &std::path::Path,
    handle: SearchHandle,
    wake: &Wake,
    stop: &AtomicBool,
) {
    let opened = pool.read(|conn| {
        Ok(SyncWorker::open_with_handle(
            conn,
            dir,
            IndexOptions::strategy_b(),
            SyncConfig::default(),
            handle,
            // 재구축 마지막의 index_progress 덮어쓰기는 쓰기 큐로 (읽기 연결로는 쓸 수 없다)
            &mut |progress| {
                let progress = progress.to_vec();
                pool.write(Priority::Low, move |c| Ok(write_progress_on(c, &progress)))
                    .map_err(|e| SyncError::Io(std::io::Error::other(e.to_string())))?
            },
        ))
    });
    let worker = match opened {
        Ok(Ok((worker, outcome))) => {
            if let Some(r) = outcome.rebuilt {
                tracing::info!(reason = ?r.reason, docs = r.docs, "search index rebuilt");
            }
            Arc::new(worker)
        }
        Ok(Err(e)) => {
            tracing::error!(%e, "cannot open search index; search is unavailable");
            return;
        }
        Err(e) => {
            tracing::error!(%e, "cannot read database for search index; search is unavailable");
            return;
        }
    };

    while !stop.load(Ordering::Relaxed) {
        let pending = pool.read(|c| {
            Ok(c.query_row(
                "SELECT EXISTS (SELECT 1 FROM search_outbox WHERE status = 'pending')",
                [],
                |r| r.get::<_, i64>(0),
            )? == 1)
        });
        match pending {
            Ok(true) => {}
            Ok(false) => {
                wake.wait(IDLE_POLL);
                continue;
            }
            Err(PoolError::WriterGone) => return,
            Err(e) => {
                tracing::warn!(%e, "cannot check the search outbox");
                wake.wait(ERROR_BACKOFF);
                continue;
            }
        }
        let (w, c) = (Arc::clone(&worker), Arc::clone(clock));
        match pool.write(Priority::Low, move |conn| Ok(w.process_batch(conn, &*c))) {
            Ok(Ok(report)) => {
                if let Some(msg) = &report.error {
                    tracing::warn!(error = %msg, retry = report.events_retry, failed = report.events_failed, "search index batch failed");
                    wake.wait(ERROR_BACKOFF);
                } else if let Some(due) = report.deferred_until_ms {
                    // 저우선 이벤트를 모으는 중 — 사용자 편집(wake)이 오면 바로 깨어난다
                    let wait = (due - clock.now_ms()).clamp(50, 2_000);
                    wake.wait(Duration::from_millis(wait as u64));
                }
                // 처리한 이벤트가 있으면 곧바로 다음 배치
            }
            Ok(Err(e)) => {
                tracing::error!(%e, "search index worker error");
                wake.wait(ERROR_BACKOFF);
            }
            Err(PoolError::WriterGone) => return,
            Err(e) => {
                tracing::warn!(%e, "cannot run the search index batch");
                wake.wait(ERROR_BACKOFF);
            }
        }
    }
}

#[cfg(test)]
#[path = "search_service_tests.rs"]
mod tests;
