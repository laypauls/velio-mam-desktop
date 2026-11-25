// Categories: Happy (결합·유사 미디어) / Boundary (빈 벡터·후보 상한 밖) / Validation (인자·커서) /
// Business rule (순위 전 필터, 중복 제외, degraded) / External (모델·인덱스 없음, 임베딩 실패)
use std::time::{Duration, Instant};

use mam_core::SharedClock;
use mam_core::query::{
    PageDirection, QueryService, SearchFilters, SearchMode, SearchRequest, SortDirection, SortField,
    SortSpec,
};
use mam_db::{DbPool, SystemClock, migrate, open};
use mam_vector::{NewVector, VectorKind, VectorSource};

use super::*;
use crate::error::ErrorCode;

const DIM: usize = 8;
const NOW: i64 = 1_700_000_000_000;

fn semantic_key() -> ModelKey {
    ModelKey {
        kind: VectorKind::Semantic,
        model_id: "e5-test".into(),
        model_version: "1".into(),
        dimension: DIM,
    }
}

fn visual_key() -> ModelKey {
    ModelKey {
        kind: VectorKind::Visual,
        model_id: "clip-test".into(),
        model_version: "1".into(),
        dimension: DIM,
    }
}

/// `e0 + t * e1` 방향의 벡터 (t 가 클수록 e0 에서 멀다). `axis >= 2` 면 e0 와 직교.
fn dir(t: f32, axis: usize) -> Vec<f32> {
    let mut v = vec![0.0; DIM];
    if axis >= 2 {
        v[axis] = 1.0;
    } else {
        v[0] = 1.0;
        v[1] = t;
    }
    v
}

struct FakeEmbedder {
    semantic: Option<ModelKey>,
    visual: Option<ModelKey>,
    query: Result<Vec<f32>, String>,
}

impl QueryEmbedder for FakeEmbedder {
    fn semantic_key(&self) -> Option<ModelKey> {
        self.semantic.clone()
    }
    fn visual_key(&self) -> Option<ModelKey> {
        self.visual.clone()
    }
    fn embed_semantic_query(&self, _text: &str) -> Result<Vec<f32>, String> {
        self.query.clone()
    }
}

fn embedder(query: Result<Vec<f32>, String>) -> Arc<FakeEmbedder> {
    Arc::new(FakeEmbedder {
        semantic: Some(semantic_key()),
        visual: Some(visual_key()),
        query,
    })
}

#[derive(Clone)]
struct Spec {
    id: i64,
    rel: String,
    media_type: &'static str,
    volume: i64,
    status: &'static str,
    /// 같은 값이면 같은 내용 해시
    hash: Option<u8>,
    visual: Vec<Vec<f32>>,
    semantic: Option<Vec<f32>>,
}

fn spec(id: i64, rel: &str) -> Spec {
    Spec {
        id,
        rel: rel.into(),
        media_type: "image",
        volume: 1,
        status: "active",
        hash: None,
        visual: vec![],
        semantic: None,
    }
}

struct Fx {
    _dir: tempfile::TempDir,
    search: Arc<SearchService>,
    store: Arc<EmbedStore>,
}

impl Drop for Fx {
    fn drop(&mut self) {
        self.search.shutdown();
    }
}

impl Fx {
    /// 자산·벡터를 만들고 활성 세대까지 구축한 뒤 색인이 따라잡을 때까지 기다린다 (임베더는 아직 주입하지 않음).
    fn new(specs: &[Spec]) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("mam.sqlite");
        let mut conn = open(&db).unwrap();
        migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
        conn.execute_batch(
            "INSERT INTO volumes (volume_id, kind, volume_key, label, last_mount_point, availability, last_seen_at) VALUES
               (1, 'fixed', 'v1', 'Main', 'C:\\Media', 'online', 0),
               (2, 'removable', 'v2', 'USB', 'E:\\', 'offline', 0);
             INSERT INTO managed_folders (folder_id, volume_id, rel_root, path_key, display_path, created_at)
               VALUES (1, 1, 'hid', 'hid', 'C:\\Media\\hid', 0);",
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        for s in specs {
            tx.execute(
                "INSERT INTO media_assets (asset_id, media_type, status, file_size, modified_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 100, ?1, 0, 0)",
                rusqlite::params![s.id, s.media_type, s.status],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
                 VALUES (?1, ?2, ?3, lower(?3), 0)",
                rusqlite::params![s.id, s.volume, s.rel],
            )
            .unwrap();
            if let Some(h) = s.hash {
                tx.execute(
                    "UPDATE media_assets SET hash_state = 'done', hash_algo = 'blake3', content_hash = ?2
                     WHERE asset_id = ?1",
                    rusqlite::params![s.id, vec![h; 32]],
                )
                .unwrap();
            }
            tx.execute(
                "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
                 VALUES (?1, 1, 'upsert', 0, 0)",
                [s.id],
            )
            .unwrap();
        }
        tx.commit().unwrap();

        let store = Arc::new(EmbedStore::new(dir.path().join("vectors")).unwrap());
        for s in specs {
            if !s.visual.is_empty() {
                let vs: Vec<NewVector> = s
                    .visual
                    .iter()
                    .enumerate()
                    .map(|(i, v)| NewVector {
                        segment_id: None,
                        source: VectorSource::Frame,
                        frame_index: Some(i as i64),
                        vector: v,
                    })
                    .collect();
                store.put(&mut conn, &visual_key(), s.id, &vs, NOW).unwrap();
            }
            if let Some(v) = &s.semantic {
                let nv = NewVector {
                    segment_id: None,
                    source: VectorSource::Text,
                    frame_index: None,
                    vector: v,
                };
                store
                    .put(&mut conn, &semantic_key(), s.id, &[nv], NOW)
                    .unwrap();
            }
        }
        for key in [visual_key(), semantic_key()] {
            if !store.asset_map(&conn, &key).unwrap().is_empty() {
                Generations::new(&store, &key)
                    .unwrap()
                    .rebuild(&conn, &store, &key)
                    .unwrap();
            }
        }
        drop(conn);

        let pool = Arc::new(DbPool::open(&db, 2).unwrap());
        let clock: SharedClock = Arc::new(SystemClock);
        let query = Arc::new(QueryService::new(Arc::clone(&pool), Arc::clone(&clock)));
        let search = SearchService::start(
            pool,
            clock,
            dir.path().join("indexes").join("tantivy"),
            query,
        );
        let expected = specs.iter().filter(|s| s.status == "active").count() as i64;
        let fx = Self {
            _dir: dir,
            search,
            store,
        };
        fx.wait_indexed(expected);
        fx
    }

    fn wait_indexed(&self, expected_total: i64) {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Ok(p) = self.search.search(&keyword(""))
                && p.total == expected_total
            {
                return;
            }
            assert!(Instant::now() < deadline, "index was not ready in time");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn inject(&self, e: Arc<FakeEmbedder>) {
        self.search.set_query_embedder(e);
        self.search.set_embed_store(Arc::clone(&self.store));
    }

    fn hybrid(
        &self,
        text: &str,
        limit: i64,
        cursor: Option<String>,
    ) -> Result<SearchPageOut, SearchFailure> {
        let mut r = keyword(text);
        r.mode = Some(SearchMode::Hybrid);
        r.sort = SortSpec {
            field: SortField::Relevance,
            direction: SortDirection::Desc,
        };
        r.limit = limit;
        r.cursor = cursor;
        self.search.search(&r).map(|p| SearchPageOut {
            ids: p.items.iter().map(|i| i.asset_id).collect(),
            next: p.next_cursor,
            degraded: p.degraded,
            total_is_estimate: p.total_is_estimate,
            total: p.total,
        })
    }

    fn similar(
        &self,
        asset: i64,
        limit: i64,
        cursor: Option<String>,
    ) -> Result<SimilarPage, SearchFailure> {
        self.search.find_similar(&SimilarRequest {
            asset_id: asset,
            limit,
            cursor,
        })
    }
}

struct SearchPageOut {
    ids: Vec<i64>,
    next: Option<String>,
    degraded: Option<bool>,
    total_is_estimate: bool,
    total: i64,
}

fn keyword(query: &str) -> SearchRequest {
    SearchRequest {
        query: query.to_string(),
        filters: SearchFilters {
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
        },
        sort: SortSpec {
            field: SortField::Name,
            direction: SortDirection::Asc,
        },
        limit: 50,
        cursor: None,
        mode: None,
        direction: None,
        at: None,
    }
}

fn code_of(e: SearchFailure) -> ErrorCode {
    crate::error::CommandError::from(e).code()
}

/// 키워드 `beach` 는 1·2 가, 의미 벡터는 3(질의와 같음)·1(가까움)이 가깝다. 4~6 은 직교.
fn semantic_specs() -> Vec<Spec> {
    let mut s: Vec<Spec> = vec![
        spec(1, "beach_one.jpg"),
        spec(2, "beach_two.jpg"),
        spec(3, "zzz_three.jpg"),
        spec(4, "zzz_four.jpg"),
        spec(5, "zzz_five.mp4"),
        spec(6, "zzz_six.mp4"),
    ];
    s[4].media_type = "video";
    s[5].media_type = "video";
    s[0].semantic = Some(dir(0.3, 0));
    s[1].semantic = Some(dir(0.0, 5));
    s[2].semantic = Some(dir(0.0, 0));
    s[3].semantic = Some(dir(0.0, 4));
    s[4].semantic = Some(dir(0.0, 3));
    s[5].semantic = Some(dir(0.0, 2));
    s
}

#[test]
fn t_p3w_30_hybrid_fuses_keyword_and_vector_candidates_and_is_not_degraded() {
    // Arrange: 키워드만으로는 [1, 2] 뿐이다
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let keyword_only = fx.search.search(&keyword("beach")).unwrap();

    // Act
    let page = fx.hybrid("beach", 50, None).unwrap();

    // Assert: 키워드 + 벡터 모두에 오른 1 이 맨 앞, 키워드에 없는 3 도 벡터로 들어온다
    assert_eq!(keyword_only.items.len(), 2);
    assert_eq!(page.degraded, None);
    assert!(page.total_is_estimate, "fused total is the candidate count");
    assert_eq!(page.ids[0], 1, "{:?}", page.ids);
    assert!(page.ids.contains(&3), "{:?}", page.ids);
    let mut sorted = page.ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), page.ids.len(), "asset_id is never duplicated");
    assert_eq!(page.total as usize, page.ids.len());
}

#[test]
fn t_p3w_31_hybrid_without_model_store_index_or_with_embedding_failure_degrades_to_keyword() {
    // Arrange
    let fx = Fx::new(&semantic_specs());
    let keyword_ids: Vec<i64> = fx
        .search
        .search(&keyword("beach"))
        .unwrap()
        .items
        .iter()
        .map(|i| i.asset_id)
        .collect();
    let mut seen = Vec::new();

    // Act / Assert: 1) 아무것도 주입되지 않음
    seen.push(("nothing injected", fx.hybrid("beach", 50, None).unwrap()));
    // 2) 저장소만 있고 모델 키 없음 (모델 미설치)
    fx.search.set_embed_store(Arc::clone(&fx.store));
    fx.search.set_query_embedder(Arc::new(FakeEmbedder {
        semantic: None,
        visual: None,
        query: Ok(dir(0.0, 0)),
    }));
    seen.push(("model missing", fx.hybrid("beach", 50, None).unwrap()));
    // 3) 질의 임베딩 실패
    fx.inject(embedder(Err("onnx failed".into())));
    seen.push(("embedding failed", fx.hybrid("beach", 50, None).unwrap()));
    // 4) 활성 인덱스 없음 (다른 모델 버전)
    fx.search.set_query_embedder(Arc::new(FakeEmbedder {
        semantic: Some(ModelKey {
            model_version: "2".into(),
            ..semantic_key()
        }),
        visual: None,
        query: Ok(dir(0.0, 0)),
    }));
    seen.push(("no active index", fx.hybrid("beach", 50, None).unwrap()));

    for (label, page) in seen {
        assert_eq!(page.degraded, Some(true), "{label}");
        assert_eq!(page.ids, keyword_ids, "{label}: keyword results are kept");
    }
}

#[test]
fn t_p3w_32_filters_are_applied_before_ranking_not_after_the_top_candidates() {
    // Arrange: 질의에 가까운 이미지 450장 + 먼(직교) 영상 2개. 후보 상한(400)보다 이미지가 많다.
    let mut specs: Vec<Spec> = (1..=450)
        .map(|i| {
            let mut s = spec(i, &format!("img_{i}.jpg"));
            s.semantic = Some(dir(i as f32 * 0.001, 0));
            s
        })
        .collect();
    for (id, axis) in [(451, 3), (452, 4)] {
        let mut s = spec(id, &format!("clip_{id}.mp4"));
        s.media_type = "video";
        s.semantic = Some(dir(0.0, axis));
        specs.push(s);
    }
    let fx = Fx::new(&specs);
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let mut r = keyword("sunset");
    r.mode = Some(SearchMode::Hybrid);
    r.filters.media_types = vec![mam_core::query::MediaKind::Video];
    r.sort = SortSpec {
        field: SortField::Relevance,
        direction: SortDirection::Desc,
    };

    // Act
    let page = fx.search.search(&r).unwrap();

    // Assert: 상위 400 을 뽑은 뒤 거르면 영상이 사라진다 -> 허용 집합 안에서 찾는다
    let mut ids: Vec<i64> = page.items.iter().map(|i| i.asset_id).collect();
    ids.sort_unstable();
    assert_eq!(ids, [451, 452]);
    assert_eq!(page.degraded, None);
}

#[test]
fn t_p3w_33_hybrid_cursor_walks_all_results_once_and_rejects_foreign_cursors() {
    // Arrange
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let all = fx.hybrid("beach", 50, None).unwrap().ids;

    // Act: 2개씩
    let mut walked = Vec::new();
    let mut cursor = None;
    loop {
        let p = fx.hybrid("beach", 2, cursor).unwrap();
        walked.extend(p.ids);
        cursor = p.next;
        if cursor.is_none() {
            break;
        }
    }
    let first = fx.hybrid("beach", 2, None).unwrap();
    let other_query = fx.hybrid("zzz", 2, first.next.clone());
    let garbage = fx.hybrid("beach", 2, Some("h1.zz".into()));
    let keyword_cursor = {
        let mut r = keyword("beach");
        r.limit = 1;
        fx.search.search(&r).unwrap().next_cursor
    };
    let foreign = fx.hybrid("beach", 2, keyword_cursor);

    // Assert
    assert_eq!(walked, all, "paging reproduces the single-page order");
    assert_eq!(code_of(other_query.err().unwrap()), ErrorCode::InvalidInput);
    assert_eq!(code_of(garbage.err().unwrap()), ErrorCode::InvalidInput);
    assert_eq!(code_of(foreign.err().unwrap()), ErrorCode::InvalidInput);
}

#[test]
fn t_p3w_34_hybrid_with_a_field_sort_or_empty_query_keeps_keyword_semantics() {
    // Arrange
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));

    // Act: 이름 정렬을 명시한 하이브리드 / 질의 없는 하이브리드(탐색)
    let mut by_name = keyword("beach");
    by_name.mode = Some(SearchMode::Hybrid);
    let sorted = fx.search.search(&by_name).unwrap();
    let mut browse = keyword("");
    browse.mode = Some(SearchMode::Hybrid);
    let browsed = fx.search.search(&browse).unwrap();

    // Assert: 필드 정렬은 키워드로 처리했음을 알리고, 탐색은 줄일 것이 없다
    assert_eq!(sorted.degraded, Some(true));
    assert_eq!(sorted.items.len(), 2);
    assert_eq!(browsed.degraded, None);
    assert_eq!(browsed.total, 6);
}

fn similar_specs() -> Vec<Spec> {
    let mut s: Vec<Spec> = (1..=8).map(|i| spec(i, &format!("p{i}.jpg"))).collect();
    // 기준 자산 1: 프레임 2개. 2 는 같은 해시 사본(가장 비슷해도 제외돼야 한다)
    s[0].visual = vec![dir(0.0, 0), dir(0.1, 0)];
    s[0].hash = Some(7);
    s[1].visual = vec![dir(0.0, 0)];
    s[1].hash = Some(7);
    s[2].visual = vec![dir(0.2, 0)];
    s[3].visual = vec![dir(0.6, 0)];
    s[4].visual = vec![dir(1.5, 0)];
    s[5].visual = vec![dir(0.0, 5)];
    // 7: 삭제된 자산, 8: 오프라인 볼륨 (오프라인도 후보)
    s[6].visual = vec![dir(0.05, 0)];
    s[6].status = "deleted";
    s[7].visual = vec![dir(0.4, 0)];
    s[7].volume = 2;
    s
}

#[test]
fn t_p3w_35_find_similar_orders_by_similarity_excluding_self_duplicates_and_inactive_assets() {
    // Arrange
    let fx = Fx::new(&similar_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    fx.set_min_similarity(0.0);

    // Act
    let page = fx.similar(1, 50, None).unwrap();

    // Assert: 3 > 8(오프라인 포함) > 4 > 5 > 6(직교). 1(자신)·2(중복)·7(삭제) 없음
    let ids: Vec<i64> = page.items.iter().map(|i| i.summary.asset_id).collect();
    assert_eq!(ids, [3, 8, 4, 5, 6]);
    assert!(page.next_cursor.is_none());
    assert!(
        page.items
            .windows(2)
            .all(|w| w[0].similarity >= w[1].similarity),
        "{:?}",
        page.items.iter().map(|i| i.similarity).collect::<Vec<_>>()
    );
    assert!(
        page.items
            .iter()
            .all(|i| (0.0..=1.0).contains(&i.similarity))
    );
    let json = serde_json::to_value(&page.items[0]).unwrap();
    assert!(
        json.get("assetId").is_some() && json.get("similarity").is_some(),
        "{json}"
    );
}

#[test]
fn t_p3w_36_find_similar_cursor_walks_every_item_once() {
    // Arrange
    let fx = Fx::new(&similar_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    fx.set_min_similarity(0.0);

    // Act
    let mut walked = Vec::new();
    let mut cursor = None;
    let mut pages = 0;
    loop {
        let p = fx.similar(1, 2, cursor).unwrap();
        walked.extend(p.items.iter().map(|i| i.summary.asset_id));
        pages += 1;
        cursor = p.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    // Assert
    assert_eq!(walked, [3, 8, 4, 5, 6]);
    assert_eq!(pages, 3);
}

#[test]
fn t_p3w_37_find_similar_rejects_bad_arguments_and_returns_empty_without_vectors() {
    // Arrange
    let fx = Fx::new(&similar_specs());

    // Act / Assert: 검증
    for (asset, limit, cursor) in [
        (0, 10, None),
        (1, 0, None),
        (1, 201, None),
        (1, 10, Some("x".to_string())),
    ] {
        let e = fx.similar(asset, limit, cursor).unwrap_err();
        assert_eq!(code_of(e), ErrorCode::InvalidInput);
    }
    // 없는/삭제된 자산
    assert_eq!(
        code_of(fx.similar(999, 10, None).unwrap_err()),
        ErrorCode::NotFound
    );
    assert_eq!(
        code_of(fx.similar(7, 10, None).unwrap_err()),
        ErrorCode::NotFound
    );
    // 모델·저장소 미주입 -> 빈 페이지
    let none = fx.similar(1, 10, None).unwrap();
    assert!(none.items.is_empty() && none.next_cursor.is_none());
    // 주입했지만 기준 자산에 시각 벡터가 없다 (자산 7 말고 벡터 없는 자산이 필요하다 -> 키만 다른 모델)
    fx.search.set_embed_store(Arc::clone(&fx.store));
    fx.search.set_query_embedder(Arc::new(FakeEmbedder {
        semantic: None,
        visual: Some(ModelKey {
            model_version: "2".into(),
            ..visual_key()
        }),
        query: Ok(vec![]),
    }));
    assert!(fx.similar(1, 10, None).unwrap().items.is_empty());
}

#[test]
fn t_p3w_30e_cached_embedding_map_is_refreshed_when_a_new_vector_is_stored() {
    // Arrange: 6 번 자산은 질의와 직교 — 첫 질의(표 캐시가 만들어진다)에서는 의미 후보가 아니다
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let first = fx.hybrid("beach", 50, None).unwrap();
    let again = fx.hybrid("beach", 50, None).unwrap();
    assert_eq!(first.ids, again.ids, "the cache does not change results");
    assert!(first.ids.iter().position(|id| *id == 6) > first.ids.iter().position(|id| *id == 3));

    // Act: 6 번에 질의와 같은 방향의 새 벡터를 저장하고 세대를 다시 만든다
    let mut conn = mam_db::open(&fx._dir.path().join("mam.sqlite")).unwrap();
    let v = dir(0.0, 0);
    let nv = NewVector {
        segment_id: None,
        source: VectorSource::Text,
        frame_index: None,
        vector: &v,
    };
    fx.store
        .put(&mut conn, &semantic_key(), 6, &[nv], NOW)
        .unwrap();
    Generations::new(&fx.store, &semantic_key())
        .unwrap()
        .rebuild(&conn, &fx.store, &semantic_key())
        .unwrap();
    let after = fx.hybrid("beach", 50, None).unwrap();

    // Assert: 새 벡터를 가리키는 표로 바뀌어 6 번이 의미 후보로 올라온다 (낡은 표라면 새 embedding_id 를 몰라 빠진다)
    let rank = |ids: &[i64], a: i64| ids.iter().position(|id| *id == a);
    assert!(
        rank(&after.ids, 6) < rank(&first.ids, 6),
        "asset 6 moved up: {:?} -> {:?}",
        first.ids,
        after.ids
    );
}

// Business rule: 시작 후 워밍업이 맵을 미리 읽어 두어도, 이후 저장된 벡터는 다음 질의에 보인다
#[test]
fn t_p3w_30f_warmed_vector_map_sees_a_vector_stored_after_the_warm_up() {
    // Arrange: 주입 전에는 아무것도 읽지 않는다 (모델·저장소 없음 -> 건너뜀)
    let fx = Fx::new(&semantic_specs());
    assert_eq!(fx.search.warm_vector_maps(), 0);
    fx.inject(embedder(Ok(dir(0.0, 0))));
    assert_eq!(fx.search.warm_vector_maps(), 2, "semantic + visual keys");
    let first = fx.hybrid("beach", 50, None).unwrap();

    // Act: 6 번에 질의와 같은 방향의 새 벡터 + 세대 재구축
    let mut conn = mam_db::open(&fx._dir.path().join("mam.sqlite")).unwrap();
    let v = dir(0.0, 0);
    let nv = NewVector {
        segment_id: None,
        source: VectorSource::Text,
        frame_index: None,
        vector: &v,
    };
    fx.store
        .put(&mut conn, &semantic_key(), 6, &[nv], NOW)
        .unwrap();
    Generations::new(&fx.store, &semantic_key())
        .unwrap()
        .rebuild(&conn, &fx.store, &semantic_key())
        .unwrap();
    let after = fx.hybrid("beach", 50, None).unwrap();

    // Assert
    let rank = |ids: &[i64], a: i64| ids.iter().position(|id| *id == a);
    assert!(rank(&after.ids, 6) < rank(&first.ids, 6));
}

// ─── P4 후속: 허용 자산 집합 (빠른 수집 + 짧은 캐시) ───────────────────────────────

/// 의미 벡터가 질의(축 0)와 같은 자산 6개 이상 + 제외돼야 하는 자산들:
/// 7 = 내용 변경으로 대체된 구버전(superseded, 벡터 보존), 8 = 파일 없음, 9 = 오프라인 볼륨.
fn allowed_specs() -> Vec<Spec> {
    let mut s = semantic_specs();
    let mut stale = spec(7, "zzz_stale.jpg");
    stale.status = "superseded";
    stale.semantic = Some(dir(0.0, 0));
    let mut gone = spec(8, "zzz_gone.jpg");
    gone.semantic = Some(dir(0.0, 0));
    let mut usb = spec(9, "zzz_usb.jpg");
    usb.volume = 2;
    usb.semantic = Some(dir(0.01, 0));
    s.extend([stale, gone, usb]);
    s
}

fn mark_missing(fx: &Fx, asset: i64) {
    fx.search
        .pool
        .write(mam_db::Priority::High, move |c| {
            Ok(c.execute_batch(&format!(
                "UPDATE file_locations SET presence = 'missing' WHERE asset_id = {asset};
                 UPDATE media_assets SET revision = revision + 1 WHERE asset_id = {asset};
                 INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
                   SELECT asset_id, revision, 'upsert', 0, 0 FROM media_assets WHERE asset_id = {asset};"
            )))
        })
        .unwrap()
        .unwrap();
    fx.search.wake();
}

fn hybrid_with(fx: &Fx, text: &str, tweak: impl Fn(&mut SearchRequest)) -> Vec<i64> {
    let mut r = keyword(text);
    r.mode = Some(SearchMode::Hybrid);
    r.sort = SortSpec {
        field: SortField::Relevance,
        direction: SortDirection::Desc,
    };
    tweak(&mut r);
    fx.search
        .search(&r)
        .unwrap()
        .items
        .iter()
        .map(|i| i.asset_id)
        .collect()
}

// Business rule: 새 경로(빠른 수집 + 캐시)는 예전 경로(전체 수집)와 같은 결과를 내고, 색인에 없는 자산의 벡터는 새지 않는다
#[test]
fn t_p4f_01_cached_allowed_set_gives_the_same_results_as_the_full_collection() {
    // Arrange: 기본 필터 / 영상만 / 오프라인 포함 / 파일 없음 포함
    let fx = Fx::new(&allowed_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    mark_missing(&fx, 8);
    let deadline = Instant::now() + Duration::from_secs(30);
    while fx.search.search(&keyword("zzz_gone")).unwrap().total != 0 {
        assert!(Instant::now() < deadline, "asset 8 was not re-indexed");
        std::thread::sleep(Duration::from_millis(50));
    }
    type Tweak = Box<dyn Fn(&mut SearchRequest)>;
    let cases: Vec<(&str, Tweak)> = vec![
        ("default", Box::new(|_| {})),
        (
            "video only",
            Box::new(|r| r.filters.media_types = vec![mam_core::query::MediaKind::Video]),
        ),
        (
            "online only",
            Box::new(|r| r.filters.include_offline = false),
        ),
        (
            "with missing",
            Box::new(|r| r.filters.include_missing = true),
        ),
    ];

    for (name, tweak) in &cases {
        // Act: 같은 질의를 예전 경로 -> 새 경로(첫 호출 = 구축, 둘째 = 캐시 적중) 순으로
        fx.search
            .vectors
            .legacy_allowed
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let legacy = hybrid_with(&fx, "beach", tweak);
        fx.search
            .vectors
            .legacy_allowed
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let built = hybrid_with(&fx, "beach", tweak);
        let cached = hybrid_with(&fx, "beach", tweak);

        // Assert
        assert_eq!(built, legacy, "{name}: built");
        assert_eq!(cached, legacy, "{name}: cached");
        assert!(!legacy.contains(&7), "{name}: superseded version leaked");
    }
    // 기본 필터(오프라인 포함, 파일 없음 제외): 8 은 빠지고 9 는 보인다. 온라인만이면 9 도 빠진다
    let default = hybrid_with(&fx, "beach", |_| {});
    assert!(default.contains(&3) && default.contains(&9) && !default.contains(&8));
    assert!(!hybrid_with(&fx, "beach", |r| r.filters.include_offline = false).contains(&9));
}

// Concurrency/Time: 같은 필터는 한 번만 구성하고, 색인이 바뀌면(파일 없음 전환) 다시 구성해 새 상태를 본다
#[test]
fn t_p4f_02_allowed_set_is_reused_per_filter_and_rebuilt_after_the_index_changes() {
    use std::sync::atomic::Ordering::Relaxed;
    // Arrange
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let builds = || fx.search.vectors.allowed.builds.load(Relaxed);
    let first = hybrid_with(&fx, "beach", |_| {});
    assert!(first.contains(&2), "asset 2 is an allowed vector candidate");
    let after_first = builds();

    // Act + Assert: 같은 필터(질의어만 다름) = 재사용, 다른 필터 = 새 구성
    hybrid_with(&fx, "sunset", |_| {});
    assert_eq!(builds(), after_first, "same filters reuse the set");
    hybrid_with(&fx, "beach", |r| r.filters.include_offline = false);
    assert_eq!(builds(), after_first + 1, "different filters build again");

    // Act: 2 번이 파일 없음으로 바뀌고 색인이 따라잡는다
    mark_missing(&fx, 2);
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let ids = hybrid_with(&fx, "beach", |_| {});
        if !ids.contains(&2) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "stale allowed set after index change"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // Assert: 도장이 바뀌어 다시 구성됐다
    assert!(
        builds() > after_first + 1,
        "builds {} after_first {after_first}",
        builds()
    );
}

// Boundary: 캐시는 항목 수를 제한한다 (CAP 초과 필터를 돌려도 결과는 같다)
#[test]
fn t_p4f_03_allowed_cache_is_bounded_and_results_do_not_depend_on_eviction() {
    use std::sync::atomic::Ordering::Relaxed;
    // Arrange
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    type Tweak = Box<dyn Fn(&mut SearchRequest)>;
    let variants: Vec<Tweak> = vec![
        Box::new(|_| {}),
        Box::new(|r| r.filters.include_offline = false),
        Box::new(|r| r.filters.include_missing = true),
        Box::new(|r| r.filters.size_min = Some(1)),
        Box::new(|r| r.filters.size_max = Some(1_000)),
        Box::new(|r| r.filters.favorites_only = false),
    ];
    let baseline: Vec<Vec<i64>> = variants
        .iter()
        .map(|t| hybrid_with(&fx, "beach", t))
        .collect();
    let builds = fx.search.vectors.allowed.builds.load(Relaxed);

    // Act: 한 바퀴 더 — 가장 오래된 필터는 밀려났으므로 다시 구성되지만 결과는 같다
    let again: Vec<Vec<i64>> = variants
        .iter()
        .map(|t| hybrid_with(&fx, "beach", t))
        .collect();

    // Assert
    assert_eq!(again, baseline);
    assert!(
        fx.search.vectors.allowed.builds.load(Relaxed) > builds,
        "eviction forced rebuilds"
    );
}

// Business rule: 시작 후 워밍업이 UI 기본 필터의 허용 집합을 미리 만들어, 첫 하이브리드 질의가 다시 만들지 않는다
#[test]
fn t_p4f_04_warm_up_prebuilds_the_allowed_set_for_the_default_filters() {
    use std::sync::atomic::Ordering::Relaxed;
    // Arrange
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    assert_eq!(fx.search.vectors.allowed.builds.load(Relaxed), 0);

    // Act
    let warmed = fx.search.warm_allowed_set(Duration::from_secs(10));
    let builds_after_warm = fx.search.vectors.allowed.builds.load(Relaxed);
    let ids = hybrid_with(&fx, "beach", |_| {});

    // Assert: 첫 질의는 만들어 둔 집합을 쓴다
    assert!(warmed);
    assert_eq!(builds_after_warm, 1);
    assert_eq!(fx.search.vectors.allowed.builds.load(Relaxed), 1);
    assert!(ids.contains(&3));
}

// External: 색인이 열리지 않으면 정해진 시간만 기다리고 포기한다 (종료를 붙잡지 않는다)
#[test]
fn t_p4f_05_warm_up_gives_up_when_the_index_never_opens() {
    // Arrange: 워커가 열 수 없는 인덱스 경로 (파일이 디렉터리 자리에 있다)
    let dir = tempfile::TempDir::new().unwrap();
    let db = dir.path().join("mam.sqlite");
    let mut conn = open(&db).unwrap();
    migrate(&mut conn, &dir.path().join("backups"), &SystemClock).unwrap();
    drop(conn);
    let pool = Arc::new(DbPool::open(&db, 2).unwrap());
    let clock: SharedClock = Arc::new(SystemClock);
    let query = Arc::new(QueryService::new(Arc::clone(&pool), Arc::clone(&clock)));
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let search = SearchService::start(pool, clock, blocked.join("tantivy"), query);

    // Act
    let started = Instant::now();
    let warmed = search.warm_allowed_set(Duration::from_millis(1500));

    // Assert
    assert!(!warmed);
    assert!(started.elapsed() < Duration::from_secs(10));
    search.shutdown();
}

// ---------------------------------------------------------------- 유사 미디어 최소 유사도 (설정 similarMinSimilarity)
// Categories: Happy (기본값 0.75) / Boundary (같음 포함·바로 아래 제외·0 = 제한 없음) / Business rule (커서 종료) /
// Concurrency (설정 변경은 다음 호출부터)

impl Fx {
    fn set_min_similarity(&self, min: f64) {
        self.search
            .pool
            .write(mam_db::Priority::High, move |c| {
                Ok(mam_db::settings::update(
                    c,
                    serde_json::json!({"similarMinSimilarity": min})
                        .as_object()
                        .unwrap(),
                )
                .is_ok())
            })
            .unwrap()
            .then_some(())
            .expect("valid min similarity");
    }

    fn similar_ids(&self, limit: i64, cursor: Option<String>) -> (Vec<i64>, Option<String>) {
        let p = self.similar(1, limit, cursor).unwrap();
        (p.items.iter().map(|i| i.summary.asset_id).collect(), p.next_cursor)
    }

    /// 제한 없이 읽은 자산 4(순위 3번째)의 유사도 — 경계 시험의 기준값
    fn similarity_of_third(&self) -> f32 {
        self.set_min_similarity(0.0);
        let p = self.similar(1, 50, None).unwrap();
        assert_eq!(p.items[2].summary.asset_id, 4);
        p.items[2].similarity
    }
}

fn fx_similar() -> Fx {
    let fx = Fx::new(&similar_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    fx
}

#[test]
fn t_simlim_01_default_min_similarity_is_75_percent() {
    // Arrange: 설정을 건드리지 않은 기본값
    let fx = fx_similar();

    // Act
    let (ids, next) = fx.similar_ids(50, None);

    // Assert: 5(약 0.63)·6(0)은 기준 미만
    assert_eq!(ids, [3, 8, 4]);
    assert!(next.is_none());
}

#[test]
fn t_simlim_02_zero_means_no_limit() {
    let fx = fx_similar();
    fx.set_min_similarity(0.0);
    assert_eq!(fx.similar_ids(50, None).0, [3, 8, 4, 5, 6]);
}

#[test]
fn t_simlim_03_item_equal_to_min_is_included_and_just_below_is_excluded() {
    // Arrange
    let fx = fx_similar();
    let s4 = fx.similarity_of_third();

    // Act / Assert: 같으면 포함
    fx.set_min_similarity(f64::from(s4));
    assert_eq!(fx.similar_ids(50, None).0, [3, 8, 4]);
    // 바로 위(다음 f32)면 4 는 빠진다
    fx.set_min_similarity(f64::from(f32::from_bits(s4.to_bits() + 1)));
    assert_eq!(fx.similar_ids(50, None).0, [3, 8]);
}

#[test]
fn t_simlim_04_threshold_reached_mid_page_returns_fewer_items_and_no_cursor() {
    let fx = fx_similar();
    let s4 = fx.similarity_of_third();
    fx.set_min_similarity(f64::from(s4));

    let (ids, next) = fx.similar_ids(4, None);

    assert_eq!(ids, [3, 8, 4]);
    assert!(next.is_none(), "paging must end at the threshold");
}

#[test]
fn t_simlim_05_threshold_reached_exactly_at_page_end_has_no_cursor() {
    let fx = fx_similar();
    let s4 = fx.similarity_of_third();
    fx.set_min_similarity(f64::from(s4));

    let (ids, next) = fx.similar_ids(3, None);

    assert_eq!(ids, [3, 8, 4]);
    assert!(next.is_none(), "next item is below the minimum");
}

#[test]
fn t_simlim_06_cursor_continuation_respects_the_threshold() {
    // Arrange
    let fx = fx_similar();
    let s4 = fx.similarity_of_third();
    fx.set_min_similarity(f64::from(s4));

    // Act
    let (first, cursor) = fx.similar_ids(2, None);
    let (second, end) = fx.similar_ids(2, cursor.clone());

    // Assert
    assert_eq!(first, [3, 8]);
    assert!(cursor.is_some());
    assert_eq!(second, [4]);
    assert!(end.is_none());
}

#[test]
fn t_simlim_07_changing_the_setting_applies_on_the_next_call() {
    let fx = fx_similar();
    assert_eq!(fx.similar_ids(50, None).0, [3, 8, 4]);
    fx.set_min_similarity(0.0);
    assert_eq!(fx.similar_ids(50, None).0, [3, 8, 4, 5, 6]);
    fx.set_min_similarity(0.99);
    assert!(fx.similar_ids(50, None).0.len() <= 1);
}

fn hybrid_request(text: &str, limit: i64) -> SearchRequest {
    let mut r = keyword(text);
    r.mode = Some(SearchMode::Hybrid);
    r.sort = SortSpec {
        field: SortField::Relevance,
        direction: SortDirection::Desc,
    };
    r.limit = limit;
    r
}

#[test]
fn t_p3w_38_hybrid_pages_support_backward_cursor_and_at_seek() {
    // Arrange
    let fx = Fx::new(&semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let all = fx.hybrid("beach", 50, None).unwrap().ids;
    assert!(all.len() >= 3, "needs at least 3 hybrid results: {all:?}");
    let ids = |p: &mam_core::query::SearchPage| p.items.iter().map(|i| i.asset_id).collect::<Vec<_>>();

    // Act: 2건씩 앞으로 간 뒤 마지막 쪽의 prevCursor 로 뒤로
    let mut r = hybrid_request("beach", 2);
    let first = fx.search.search(&r).unwrap();
    r.cursor = first.next_cursor.clone();
    let second = fx.search.search(&r).unwrap();
    let mut back = hybrid_request("beach", 2);
    back.cursor = second.prev_cursor.clone();
    back.direction = Some(PageDirection::Backward);
    let before = fx.search.search(&back).unwrap();
    let mut at = hybrid_request("beach", 2);
    at.at = Some(1);
    let seek = fx.search.search(&at).unwrap();
    let mut past = hybrid_request("beach", 2);
    past.at = Some(10_000);
    let beyond = fx.search.search(&past).unwrap();

    // Assert
    assert_eq!((first.start_offset, first.prev_cursor.is_none()), (0, true));
    assert_eq!(second.start_offset, 2);
    assert_eq!(ids(&before), ids(&first), "backward from page 2 = page 1");
    assert_eq!(before.start_offset, 0);
    assert_eq!(ids(&seek), all[1..3.min(all.len())], "at=1");
    assert_eq!(seek.start_offset, 1);
    assert!(seek.prev_cursor.is_some());
    assert!(beyond.items.is_empty() && beyond.next_cursor.is_none() && beyond.prev_cursor.is_none());
    assert_eq!(beyond.start_offset, beyond.total, "past the end clamps to the candidate count");
    assert!(first.total_is_estimate);
    assert_eq!(first.generation, second.generation);

    // 실패 경로: at + cursor, backward 인데 커서 없음
    let mut both = hybrid_request("beach", 2);
    both.cursor = first.next_cursor;
    both.at = Some(0);
    let mut orphan = hybrid_request("beach", 2);
    orphan.direction = Some(PageDirection::Backward);
    assert_eq!(code_of(fx.search.search(&both).unwrap_err()), ErrorCode::InvalidInput);
    assert_eq!(code_of(fx.search.search(&orphan).unwrap_err()), ErrorCode::InvalidInput);
}

// Business rule: 별점 범위 필터는 하이브리드에서도 후보 상한(400) 이전의 허용 집합에 적용된다 (후필터 아님)
#[test]
fn t_search_rating_h01_hybrid_applies_the_rating_range_before_ranking() {
    // Arrange: 질의에 가까운 이미지 450장 + 먼 영상 2개(별점 5)
    let mut specs: Vec<Spec> = (1..=450)
        .map(|i| {
            let mut s = spec(i, &format!("img_{i}.jpg"));
            s.semantic = Some(dir(i as f32 * 0.001, 0));
            s
        })
        .collect();
    for (id, axis) in [(451, 3), (452, 4)] {
        let mut s = spec(id, &format!("clip_{id}.mp4"));
        s.media_type = "video";
        s.semantic = Some(dir(0.0, axis));
        specs.push(s);
    }
    let fx = Fx::new(&specs);
    fx.inject(embedder(Ok(dir(0.0, 0))));
    fx.search
        .pool
        .write(mam_db::Priority::High, |c| {
            Ok(c.execute_batch(
                "INSERT INTO asset_user_data (asset_id, rating, updated_at) VALUES (451, 5, 0), (452, 5, 0);
                 UPDATE media_assets SET revision = revision + 1 WHERE asset_id IN (451, 452);
                 INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
                   SELECT asset_id, revision, 'upsert', 0, 0 FROM media_assets WHERE asset_id IN (451, 452);",
            ))
        })
        .unwrap()
        .unwrap();
    fx.search.wake();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let mut r = keyword("");
        r.filters.rating_min = Some(5);
        if fx.search.search(&r).map(|p| p.total).unwrap_or(0) == 2 {
            break;
        }
        assert!(Instant::now() < deadline, "rating edit was not indexed");
        std::thread::sleep(Duration::from_millis(50));
    }

    // Act
    let mut rated = hybrid_with(&fx, "sunset", |r| {
        r.filters.rating_min = Some(5);
        r.filters.rating_max = Some(5);
    });
    let unrated = hybrid_with(&fx, "sunset", |r| {
        r.filters.rating_min = Some(0);
        r.filters.rating_max = Some(0);
        r.limit = 200;
    });

    // Assert: 상위 400 을 뽑은 뒤 거르면 영상이 사라진다 -> 허용 집합 안에서 찾는다
    rated.sort_unstable();
    assert_eq!(rated, [451, 452]);
    assert!(!unrated.contains(&451) && !unrated.contains(&452) && !unrated.is_empty());
}

#[path = "search_hide_hyb_tests.rs"]
mod hide;
