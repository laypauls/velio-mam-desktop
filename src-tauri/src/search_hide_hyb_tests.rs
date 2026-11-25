// 숨김 폴더 제외 (하이브리드·비슷한 항목) - search_hybrid_tests 의 픽스처를 쓰는 하위 모듈.
use super::*;

// Categories: Business rule (숨김 폴더는 키워드·의미·하이브리드·비슷한 항목에서 제외, 폴더 범위는 예외) /
// Concurrency (토글 후 허용 집합 캐시 무효화, 이전 커서 stale)
/// 관리 폴더 1(`hid/`) 로 옮긴다 - 색인 전에 정해야 하므로 스펙 단계에서 경로를 바꾼다.
fn in_hidden_dir(s: &mut Spec) {
    s.rel = format!("hid/{}", s.rel);
}

fn set_folder_hidden(fx: &Fx, hidden: bool) {
    fx.search
        .pool
        .write(mam_db::Priority::High, move |c| {
            Ok(mam_core::scan::set_hidden(c, 1, hidden))
        })
        .unwrap()
        .unwrap();
}

fn hidden_semantic_specs() -> Vec<Spec> {
    let mut s = semantic_specs();
    in_hidden_dir(&mut s[0]); // 1: 키워드 + 의미
    in_hidden_dir(&mut s[2]); // 3: 의미로만 닿는다
    s
}

#[test]
fn t_hide_hyb_01_hybrid_semantic_and_keyword_paths_exclude_the_hidden_folder_and_toggle_invalidates_the_cache()
 {
    // Arrange
    let fx = Fx::new(&hidden_semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let before = hybrid_with(&fx, "beach", |_| {});
    assert!(
        before.contains(&1) && before.contains(&3),
        "기준: {before:?}"
    );

    // Act
    set_folder_hidden(&fx, true);
    let hybrid = hybrid_with(&fx, "beach", |_| {});
    let keyword_only = fx.search.search(&keyword("beach")).unwrap();

    // Assert
    assert!(!hybrid.contains(&1) && !hybrid.contains(&3), "{hybrid:?}");
    assert!(hybrid.contains(&2), "다른 폴더(밖) 자산은 유지: {hybrid:?}");
    assert_eq!(
        keyword_only
            .items
            .iter()
            .map(|i| i.asset_id)
            .collect::<Vec<_>>(),
        [2]
    );

    // Act: 해제하면 허용 집합이 다시 구성되어 복귀한다
    set_folder_hidden(&fx, false);
    let restored = hybrid_with(&fx, "beach", |_| {});
    assert_eq!(restored, before);
}

#[test]
fn t_hide_hyb_02_folder_scope_shows_hidden_items_in_hybrid_and_keyword() {
    // Arrange
    let fx = Fx::new(&hidden_semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    set_folder_hidden(&fx, true);
    let scope = |r: &mut SearchRequest| {
        r.filters.folder = Some(mam_core::query::FolderScope {
            folder_id: 1,
            rel_path: None,
            recursive: true,
        });
    };

    // Act
    let mut hybrid = hybrid_with(&fx, "beach", scope);
    hybrid.sort_unstable();
    let mut kw = keyword("beach");
    scope(&mut kw);
    let kw_ids: Vec<i64> = fx
        .search
        .search(&kw)
        .unwrap()
        .items
        .iter()
        .map(|i| i.asset_id)
        .collect();

    // Assert: 범위를 지정하면 숨김 폴더 내용만 보인다 (의미 경로로만 닿는 3 포함)
    assert_eq!(hybrid, [1, 3]);
    assert_eq!(kw_ids, [1]);
}

#[test]
fn t_hide_hyb_03_find_similar_excludes_hidden_folder_candidates() {
    // Arrange: 3 은 기준(1)과 가장 비슷한 후보인데 숨김 폴더에 있다
    let mut specs = similar_specs();
    in_hidden_dir(&mut specs[2]);
    let fx = Fx::new(&specs);
    fx.inject(embedder(Ok(dir(0.0, 0))));
    fx.set_min_similarity(0.0);
    let ids = |fx: &Fx| -> Vec<i64> {
        fx.similar(1, 50, None)
            .unwrap()
            .items
            .iter()
            .map(|i| i.summary.asset_id)
            .collect()
    };
    assert_eq!(ids(&fx), [3, 8, 4, 5, 6], "기준");

    // Act
    set_folder_hidden(&fx, true);

    // Assert
    assert_eq!(ids(&fx), [8, 4, 5, 6]);
    set_folder_hidden(&fx, false);
    assert_eq!(ids(&fx), [3, 8, 4, 5, 6], "해제하면 복귀");
}

#[test]
fn t_hide_hyb_04_a_hybrid_cursor_issued_before_the_toggle_is_rejected_as_stale() {
    // Arrange
    let fx = Fx::new(&hidden_semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let cursor = fx.hybrid("beach", 1, None).unwrap().next.unwrap();
    assert!(
        fx.hybrid("beach", 1, Some(cursor.clone())).is_ok(),
        "같은 상태에서는 유효"
    );

    // Act
    set_folder_hidden(&fx, true);

    // Assert
    let err = fx
        .hybrid("beach", 1, Some(cursor))
        .err()
        .expect("stale cursor");
    assert_eq!(code_of(err), ErrorCode::InvalidInput);
}

#[test]
fn t_hide_hyb_05_hybrid_with_a_collection_shows_hidden_folder_items() {
    // Arrange: 컬렉션에 숨김 폴더의 자산(1, 3)과 밖의 자산(2)을 담는다
    use mam_core::userdata::UserData;
    let fx = Fx::new(&hidden_semantic_specs());
    fx.inject(embedder(Ok(dir(0.0, 0))));
    let ud = UserData::new(Arc::clone(&fx.search.pool), Arc::new(SystemClock));
    let cid = ud.create_collection("숨김 포함").unwrap().collection_id;
    ud.add_to_collection(cid, &[1, 2, 3]).unwrap();
    fx.search.wake();
    let in_collection = |r: &mut SearchRequest| r.filters.collection_id = Some(cid);
    let deadline = Instant::now() + Duration::from_secs(10);
    while {
        let mut r = keyword("");
        in_collection(&mut r);
        fx.search.search(&r).unwrap().total != 3
    } {
        assert!(Instant::now() < deadline, "collection was not indexed");
        std::thread::sleep(Duration::from_millis(20));
    }

    // Act
    set_folder_hidden(&fx, true);
    let mut ids = hybrid_with(&fx, "beach", in_collection);
    ids.sort_unstable();

    // Assert: 컬렉션 범위에서는 숨김 폴더의 1·3 도 보인다 (3 은 의미 경로로만 닿는다)
    assert!(
        ids.contains(&1) && ids.contains(&2) && ids.contains(&3),
        "{ids:?}"
    );
    assert!(
        !hybrid_with(&fx, "beach", |_| {}).contains(&1),
        "컬렉션 밖에서는 숨김"
    );
}
