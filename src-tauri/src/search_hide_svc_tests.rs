// 숨김 폴더 제외 (서비스 계층) - search_service_tests 의 픽스처를 쓰는 하위 모듈.
use super::*;

// Categories: Business rule (숨김 폴더 제외 / 폴더·컬렉션 범위 예외) / Concurrency (토글 전 커서는 stale) /
// Boundary (숨김 없음 = 변화 없음, 등록 해제 중 폴더는 제외 목록에 없음)
fn set_hidden_flag(fx: &Fx, folder_id: i64, hidden: bool) {
    fx.pool
        .write(mam_db::Priority::High, move |c| {
            Ok(mam_core::scan::set_hidden(c, folder_id, hidden))
        })
        .unwrap()
        .unwrap();
}

fn sorted_ids(fx: &Fx, r: &SearchRequest) -> Vec<i64> {
    let mut v = ids_of(&fx.search.search(r).unwrap());
    v.sort_unstable();
    v
}

#[test]
fn t_hide_svc_01_hidden_folder_is_excluded_from_keyword_and_filter_only_search_without_reindex() {
    // Arrange: 폴더 1 = 자산 1·2, 자산 3 은 관리 폴더 밖(USB)
    let fx = fixture();
    ready(&fx, 3);
    assert_eq!(sorted_ids(&fx, &request("")), [1, 2, 3], "숨김 없음 = 변화 없음");

    // Act
    set_hidden_flag(&fx, 1, true);

    // Assert: 색인 대기 없이 즉시 반영 (질의 시점 필터)
    assert_eq!(sorted_ids(&fx, &request("")), [3], "텍스트 없는 질의");
    assert_eq!(sorted_ids(&fx, &request("beach")), Vec::<i64>::new(), "키워드 질의");
    assert_eq!(sorted_ids(&fx, &request("forest")), [3], "다른 폴더 자산은 유지");
    assert_eq!(fx.search.search(&request("")).unwrap().total, 1);

    // Act: 해제
    set_hidden_flag(&fx, 1, false);
    assert_eq!(sorted_ids(&fx, &request("")), [1, 2, 3]);
}

#[test]
fn t_hide_svc_02_folder_scope_and_collection_scope_still_show_hidden_folder_items() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let ud = UserData::new(Arc::clone(&fx.pool), Arc::new(SystemClock));
    let cid = ud.create_collection("숨김 포함").unwrap().collection_id;
    ud.add_to_collection(cid, &[1, 3]).unwrap();
    fx.search.wake();
    let mut in_collection = request("");
    in_collection.filters.collection_id = Some(cid);
    wait_total(&fx, &in_collection, 2);
    let mut scoped = request("");
    scoped.filters.folder = Some(FolderScope {
        folder_id: 1,
        rel_path: None,
        recursive: true,
    });

    // Act
    set_hidden_flag(&fx, 1, true);

    // Assert
    assert_eq!(sorted_ids(&fx, &scoped), [1, 2], "폴더 범위 지정 시 숨김 폴더 내용 표시");
    assert_eq!(sorted_ids(&fx, &in_collection), [1, 3], "컬렉션 보기는 숨김 폴더 항목도 표시");
    assert_eq!(sorted_ids(&fx, &request("")), [3], "그 외 보기는 숨김");
}

#[test]
fn t_hide_svc_03_a_cursor_issued_before_the_toggle_is_stale_after_it() {
    // Arrange
    let fx = fixture();
    ready(&fx, 3);
    let mut first = request("");
    first.limit = 1;
    let cursor = fx.search.search(&first).unwrap().next_cursor.unwrap();
    let mut next = first.clone();
    next.cursor = Some(cursor);
    assert!(fx.search.search(&next).is_ok(), "같은 상태에서는 커서 유효");

    // Act
    set_hidden_flag(&fx, 1, true);

    // Assert
    assert_eq!(code_of(fx.search.search(&next).unwrap_err()), ErrorCode::InvalidInput);
    assert_eq!(ids_of(&fx.search.search(&first).unwrap()), [3], "처음부터 다시 조회하면 정상");
}

#[test]
fn t_hide_svc_04_a_folder_being_removed_stays_excluded_so_its_docs_do_not_reappear_during_detach() {
    // Arrange: 등록 해제 중(enabled = 0)이어도 detach 가 끝날 때까지 색인 문서가 남아 있으므로 계속 제외한다
    let fx = fixture();
    ready(&fx, 3);
    fx.pool
        .write(mam_db::Priority::High, |c| {
            Ok(c.execute("UPDATE managed_folders SET hidden = 1, enabled = 0 WHERE folder_id = 1", []))
        })
        .unwrap()
        .unwrap();

    // Act + Assert
    assert_eq!(fx.search.query.hidden_folder_ids().unwrap(), [1]);
    assert_eq!(sorted_ids(&fx, &request("")), [3]);
}
