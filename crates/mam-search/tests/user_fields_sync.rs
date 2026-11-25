//! 사용자 정의 메타데이터 필드 색인 — 실제 SQLite(마이그레이션 적용) + 임시 디렉터리 Tantivy (작업 문서 step 3).
//! mam-core 는 mam-search 의 의존자라 dev-dependency 로 쓸 수 없다 — 쓰기 규칙(값 변경 + revision 증가 + Outbox 를
//! 한 트랜잭션)은 SQL 로 모사한다.
//! 분류: T-umsync-01 Happy(재구축 = 증분 = SQLite) / 02 Concurrency(멱등·revision 역전) / 03 Business rule(선택지·필드 삭제 반영) /
//! 04 Business rule(삭제 중 제외) / 05 Business rule(이름 변경은 재색인 없음) / 06 Business rule(형식 버전) / 07 Boundary(쿼리 계획)

mod common;

use common::small;
use mam_db::Clock;
use mam_search::sync::user_values::{ALL_VALUES_SQL, VALUES_BY_IDS_SQL, load_user_values};
use mam_search::sync::{RebuildReason, SyncConfig, SyncWorker, load as sync_load};
use mam_search::{
    FilenameStrategy, Filters, IndexOptions, SearchIndex, UserFieldCondition as C, UserFieldValue,
    UserValue as V,
};
use rusqlite::{Connection, params};
use tempfile::{TempDir, tempdir};

struct LateClock;
impl Clock for LateClock {
    fn now_ms(&self) -> i64 {
        1_000_000
    }
}

const KW: i64 = 1;
const NUM: i64 = 2;
const DATE: i64 = 3;
const BOOL: i64 = 4;
const ENUM: i64 = 5;
const MULTI: i64 = 6;
// option ids (AUTOINCREMENT 로 만들어지는 값과 같게 1..)
const ENUM_RED: i64 = 1;
const ENUM_BLUE: i64 = 2;
const MULTI_A: i64 = 3;
const MULTI_B: i64 = 4;

struct Env {
    tmp: TempDir,
    conn: Connection,
}

impl Env {
    /// 마이그레이션 적용 + 필드 6종(타입별 1개)과 선택지 4개.
    fn new() -> Self {
        let tmp = tempdir().unwrap();
        let mut conn = mam_db::open(&tmp.path().join("t.db")).unwrap();
        mam_db::migrate(&mut conn, &tmp.path().join("backups"), &mam_db::SystemClock).unwrap();
        conn.execute(
            "INSERT INTO volumes (volume_id, kind, volume_key, last_seen_at) VALUES (1, 'fixed', 'v1', 0)",
            [],
        )
        .unwrap();
        for (i, (name, ty)) in [
            ("keyword", "keyword"),
            ("number", "number"),
            ("date", "date"),
            ("bool", "boolean"),
            ("enum", "enum"),
            ("multi", "multi_enum"),
        ]
        .iter()
        .enumerate()
        {
            conn.execute(
                "INSERT INTO metadata_fields (field_id, name, name_key, value_type, position, created_at, updated_at)
                 VALUES (?1, ?2, ?2, ?3, ?1, 0, 0)",
                params![i as i64 + 1, name, ty],
            )
            .unwrap();
        }
        for (opt, field, label) in [(1, ENUM, "red"), (2, ENUM, "blue"), (3, MULTI, "a"), (4, MULTI, "b")] {
            conn.execute(
                "INSERT INTO metadata_field_options (option_id, field_id, label, label_key, position)
                 VALUES (?1, ?2, ?3, ?3, ?1)",
                params![opt, field, label],
            )
            .unwrap();
        }
        Self { tmp, conn }
    }

    fn index_dir(&self) -> std::path::PathBuf {
        self.tmp.path().join("idx")
    }

    fn worker(&self) -> SyncWorker {
        let index = SearchIndex::open_or_create(&self.index_dir(), small(IndexOptions::all())).unwrap();
        SyncWorker::new(index, &self.index_dir(), small(IndexOptions::all()), SyncConfig::default())
    }
}

fn add_asset(conn: &Connection, rel_path: &str) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    tx.execute(
        "INSERT INTO media_assets (media_type, file_size, modified_at, created_at, updated_at)
         VALUES ('image', 1, 0, 0, 0)",
        [],
    )
    .unwrap();
    let id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO file_locations (asset_id, volume_id, rel_path, path_key, last_seen_at)
         VALUES (?1, 1, ?2, lower(?2), 0)",
        params![id, rel_path],
    )
    .unwrap();
    outbox(&tx, id as u64, 1, 0);
    tx.commit().unwrap();
    id as u64
}

fn outbox(conn: &Connection, asset_id: u64, revision: u64, priority: i64) {
    conn.execute(
        "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
         VALUES (?1, ?2, 'upsert', ?3, 1000)",
        params![asset_id as i64, revision as i64, priority],
    )
    .unwrap();
}

/// 값 변경과 같은 트랜잭션에서 revision 증가 + Outbox (mam-core 의 쓰기 규칙). 새 revision 반환.
fn bump(conn: &Connection, asset_id: u64, priority: i64) -> u64 {
    conn.execute(
        "UPDATE media_assets SET revision = revision + 1 WHERE asset_id = ?1",
        [asset_id as i64],
    )
    .unwrap();
    let rev: i64 = conn
        .query_row("SELECT revision FROM media_assets WHERE asset_id = ?1", [asset_id as i64], |r| r.get(0))
        .unwrap();
    outbox(conn, asset_id, rev as u64, priority);
    rev as u64
}

enum Val<'a> {
    Text(&'a str),
    Real(f64),
    Int(i64),
}

/// ord: 단일값 필드 0, multi_enum 은 option_id.
fn set(conn: &Connection, asset_id: u64, field: i64, ord: i64, v: Val) {
    let (t, i, r): (Option<&str>, Option<i64>, Option<f64>) = match v {
        Val::Text(s) => (Some(s), None, None),
        Val::Int(x) => (None, Some(x), None),
        Val::Real(x) => (None, None, Some(x)),
    };
    conn.execute(
        "INSERT OR REPLACE INTO asset_metadata_values (asset_id, field_id, ord, value_text, value_int, value_real, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
        params![asset_id as i64, field, ord, t, i, r],
    )
    .unwrap();
}

/// 값 쓰기 + revision/outbox 를 한 트랜잭션으로.
fn edit(conn: &Connection, asset_id: u64, f: impl FnOnce(&Connection)) -> u64 {
    let tx = conn.unchecked_transaction().unwrap();
    f(&tx);
    let rev = bump(&tx, asset_id, 0);
    tx.commit().unwrap();
    rev
}

/// 자산 1..: 1 = 모든 타입, 2 = 일부, 3 = 값 없음, 4 = 비활성(superseded, 값은 남아 있음).
fn seed(env: &Env) -> [u64; 4] {
    let a = add_asset(&env.conn, "a.jpg");
    let b = add_asset(&env.conn, "b.jpg");
    let c = add_asset(&env.conn, "c.jpg");
    let sup = add_asset(&env.conn, "old.jpg");
    // 비활성 자산을 맨 앞 id 가 아니라 중간에 두고 싶지만 id 는 증가 순이므로 값만 먼저 준다
    edit(&env.conn, a, |t| {
        set(t, a, KW, 0, Val::Text("Alpha"));
        set(t, a, NUM, 0, Val::Real(3.0));
        set(t, a, DATE, 0, Val::Int(20261115));
        set(t, a, BOOL, 0, Val::Int(1));
        set(t, a, ENUM, 0, Val::Int(ENUM_RED));
        set(t, a, MULTI, MULTI_A, Val::Int(MULTI_A));
        set(t, a, MULTI, MULTI_B, Val::Int(MULTI_B));
    });
    edit(&env.conn, b, |t| {
        set(t, b, KW, 0, Val::Text("beta"));
        set(t, b, ENUM, 0, Val::Int(ENUM_BLUE));
    });
    edit(&env.conn, sup, |t| {
        set(t, sup, KW, 0, Val::Text("alpha"));
        set(t, sup, NUM, 0, Val::Real(3.0));
        t.execute("UPDATE media_assets SET status = 'superseded' WHERE asset_id = ?1", [sup as i64]).unwrap();
    });
    [a, b, c, sup]
}

fn ids(w: &SyncWorker, conds: Vec<C>) -> Vec<u64> {
    let h = w.search_handle();
    let mut v = h.filtered_asset_ids(&Filters { user_fields: conds, ..Filters::default() }).unwrap();
    v.sort_unstable();
    v
}

/// 모든 필드·연산을 한 번씩 훑는 질의 묶음 (재구축 인덱스와 증분 인덱스가 같은지 비교하는 지문).
fn battery() -> Vec<C> {
    vec![
        C::KeywordAnyOf { field_id: KW as u64, values: vec!["alpha".into()] },
        C::KeywordAnyOf { field_id: KW as u64, values: vec!["BETA".into(), "alpha".into()] },
        C::NumberRange { field_id: NUM as u64, min: Some(3.0), max: Some(3.0) },
        C::DateRange { field_id: DATE as u64, min: Some(20261101), max: None },
        C::Bool { field_id: BOOL as u64, value: true },
        C::EnumAnyOf { field_id: ENUM as u64, option_ids: vec![ENUM_RED as u64] },
        C::EnumAnyOf { field_id: ENUM as u64, option_ids: vec![ENUM_BLUE as u64] },
        C::MultiEnum { field_id: MULTI as u64, option_ids: vec![MULTI_A as u64, MULTI_B as u64], all: true },
        C::Has { field_id: KW as u64 },
        C::Missing { field_id: KW as u64 },
        C::Missing { field_id: MULTI as u64 },
    ]
}

fn all_results(w: &SyncWorker) -> Vec<Vec<u64>> {
    battery().into_iter().map(|c| ids(w, vec![c])).collect()
}

#[test]
fn t_umsync_01_rebuild_incremental_and_loader_all_agree_with_sqlite() {
    // Arrange
    let env = Env::new();
    let [a, b, c, sup] = seed(&env);

    // Act: 증분(Outbox) 경로
    let inc = env.worker();
    inc.drain(&env.conn, &LateClock).unwrap();
    // 재구축(스트림 병합) 경로 — 증분 인덱스와 다른 디렉터리
    let tmp2 = tempdir().unwrap();
    let dir2 = tmp2.path().join("idx2");
    let index2 = SearchIndex::open_or_create(&dir2, small(IndexOptions::all())).unwrap();
    let mut rebuilt = SyncWorker::new(index2, &dir2, small(IndexOptions::all()), SyncConfig::default());
    rebuilt.rebuild(&env.conn).unwrap();

    // Assert: 기대값은 SQLite 에서 직접 계산한 것
    let expected: Vec<Vec<u64>> = vec![
        vec![a],    // alpha (비활성 sup 의 alpha 는 문서가 아님)
        vec![a, b], // beta | alpha
        vec![a],    // number == 3
        vec![a],    // date >= 20261101
        vec![a],    // bool true
        vec![a],    // enum red
        vec![b],    // enum blue
        vec![a],    // multi all(a, b)
        vec![a, b], // has keyword
        vec![c],    // missing keyword
        vec![b, c], // missing multi
    ];
    assert_eq!(all_results(&inc), expected, "incremental");
    assert_eq!(all_results(&rebuilt), expected, "rebuild");
    assert!(ids(&inc, vec![C::Has { field_id: NUM as u64 }]).iter().all(|id| *id != sup));
    // 로더가 읽은 값 = SQLite 행 (타입은 값 컬럼이 정한다)
    let docs = sync_load::load_docs(&env.conn, &[a, b, c, sup]).unwrap();
    let mut got = docs[&a].user_fields.clone();
    got.sort_by_key(|v| (v.field_id, format!("{:?}", v.value)));
    let uv = |field_id: i64, value| UserFieldValue { field_id: field_id as u64, value };
    assert_eq!(
        got,
        vec![
            uv(KW, V::Keyword("Alpha".into())),
            uv(NUM, V::Number(3.0)),
            uv(DATE, V::Int(20261115)),
            uv(BOOL, V::Int(1)),
            uv(ENUM, V::Int(ENUM_RED)),
            uv(MULTI, V::Int(MULTI_A)),
            uv(MULTI, V::Int(MULTI_B)),
        ]
    );
    assert!(docs[&c].user_fields.is_empty());
    assert!(!docs.contains_key(&sup), "inactive asset is not a document");
}

#[test]
fn t_umsync_02_duplicate_and_older_revision_events_do_not_change_the_index() {
    // Arrange
    let env = Env::new();
    let [a, ..] = seed(&env);
    let w = env.worker();
    w.drain(&env.conn, &LateClock).unwrap();
    let before = all_results(&w);
    // 값 수정 (keyword alpha -> gamma)
    let rev = edit(&env.conn, a, |t| set(t, a, KW, 0, Val::Text("Gamma")));
    w.drain(&env.conn, &LateClock).unwrap();
    let after = all_results(&w);
    assert_ne!(before, after, "edit is reflected");
    assert_eq!(ids(&w, vec![C::KeywordAnyOf { field_id: KW as u64, values: vec!["gamma".into()] }]), [a]);

    // Act: 같은 revision 의 중복 이벤트 + 더 오래된 revision 의 늦은 이벤트
    outbox(&env.conn, a, rev, 0);
    outbox(&env.conn, a, rev - 1, 0);
    let r = w.drain(&env.conn, &LateClock).unwrap();

    // Assert: 모두 건너뛰고 인덱스는 그대로
    assert_eq!(r.upserted, 0, "{r:?}");
    assert_eq!(r.skipped_stale, 1, "{r:?}");
    assert_eq!(all_results(&w), after);
    assert_eq!(ids(&w, vec![C::KeywordAnyOf { field_id: KW as u64, values: vec!["alpha".into()] }]), Vec::<u64>::new());
}

#[test]
fn t_umsync_03_removed_option_values_and_deleted_field_disappear_after_reindex() {
    // Arrange: a = enum red + multi(a, b), b = enum blue
    let env = Env::new();
    let [a, b, ..] = seed(&env);
    let w = env.worker();
    w.drain(&env.conn, &LateClock).unwrap();
    let red = C::EnumAnyOf { field_id: ENUM as u64, option_ids: vec![ENUM_RED as u64] };
    let multi_b = C::MultiEnum { field_id: MULTI as u64, option_ids: vec![MULTI_B as u64], all: false };
    assert_eq!(ids(&w, vec![red.clone()]), [a]);

    // Act 1: 선택지 삭제 — 같은 트랜잭션에서 값 정리 + 대상 자산 revision/outbox (priority 1)
    let tx = env.conn.unchecked_transaction().unwrap();
    tx.execute("DELETE FROM asset_metadata_values WHERE field_id = ?1 AND value_int = ?2", params![ENUM, ENUM_RED]).unwrap();
    tx.execute("DELETE FROM asset_metadata_values WHERE field_id = ?1 AND value_int = ?2", params![MULTI, MULTI_B]).unwrap();
    tx.execute("DELETE FROM metadata_field_options WHERE option_id IN (?1, ?2)", params![ENUM_RED, MULTI_B]).unwrap();
    bump(&tx, a, 1);
    tx.commit().unwrap();
    w.drain(&env.conn, &LateClock).unwrap();

    // Assert 1: 지워진 선택지 값은 질의에서 사라지고, 같은 필드의 다른 값은 남는다
    assert_eq!(ids(&w, vec![red]), Vec::<u64>::new());
    assert_eq!(ids(&w, vec![multi_b]), Vec::<u64>::new());
    assert_eq!(ids(&w, vec![C::Has { field_id: ENUM as u64 }]), [b], "a lost its only enum value");
    assert_eq!(
        ids(&w, vec![C::MultiEnum { field_id: MULTI as u64, option_ids: vec![MULTI_A as u64], all: false }]),
        [a]
    );

    // Act 2: 필드 삭제 (CASCADE 로 값·선택지 삭제) + 대상 자산 재색인
    let tx = env.conn.unchecked_transaction().unwrap();
    tx.execute("DELETE FROM metadata_fields WHERE field_id = ?1", [KW]).unwrap();
    bump(&tx, a, 1);
    bump(&tx, b, 1);
    tx.commit().unwrap();
    w.drain(&env.conn, &LateClock).unwrap();

    // Assert 2
    assert_eq!(ids(&w, vec![C::Has { field_id: KW as u64 }]), Vec::<u64>::new());
    assert_eq!(
        ids(&w, vec![C::KeywordAnyOf { field_id: KW as u64, values: vec!["beta".into()] }]),
        Vec::<u64>::new()
    );
    // 키워드 검색 텍스트에서도 사라진다
    let h = w.search_handle();
    assert!(h.keyword_candidates("beta", FilenameStrategy::A, &Filters::default(), 5).unwrap().is_empty());
}

#[test]
fn t_umsync_04_fields_and_options_being_deleted_are_excluded_before_their_values_are_gone() {
    // Arrange: 값은 그대로 두고 삭제 표시만 한 상태 (청크 삭제가 아직 진행 중)
    let env = Env::new();
    let [a, b, ..] = seed(&env);
    env.conn.execute("UPDATE metadata_fields SET deleting_at = 5 WHERE field_id = ?1", [KW]).unwrap();
    env.conn.execute("UPDATE metadata_field_options SET deleting_at = 5 WHERE option_id = ?1", [ENUM_RED]).unwrap();
    env.conn.execute("UPDATE metadata_field_options SET deleting_at = 5 WHERE option_id = ?1", [MULTI_B]).unwrap();

    // Act: 로더(증분)와 재구축 양쪽
    let docs = sync_load::load_docs(&env.conn, &[a, b]).unwrap();
    let inc = env.worker();
    inc.drain(&env.conn, &LateClock).unwrap();
    let tmp2 = tempdir().unwrap();
    let dir2 = tmp2.path().join("idx2");
    let mut rebuilt = SyncWorker::new(
        SearchIndex::open_or_create(&dir2, small(IndexOptions::all())).unwrap(),
        &dir2,
        small(IndexOptions::all()),
        SyncConfig::default(),
    );
    rebuilt.rebuild(&env.conn).unwrap();

    // Assert
    let fields_of = |id: u64| -> Vec<(u64, V)> {
        docs[&id].user_fields.iter().map(|v| (v.field_id, v.value.clone())).collect()
    };
    assert!(fields_of(a).iter().all(|(f, _)| *f != KW as u64), "deleting field excluded");
    assert!(!fields_of(a).contains(&(ENUM as u64, V::Int(ENUM_RED))), "deleting option excluded");
    assert!(!fields_of(a).contains(&(MULTI as u64, V::Int(MULTI_B))));
    assert!(fields_of(a).contains(&(MULTI as u64, V::Int(MULTI_A))), "other options stay");
    assert!(fields_of(b).contains(&(ENUM as u64, V::Int(ENUM_BLUE))));
    for w in [&inc, &rebuilt] {
        assert_eq!(ids(w, vec![C::Has { field_id: KW as u64 }]), Vec::<u64>::new());
        assert_eq!(ids(w, vec![C::Has { field_id: ENUM as u64 }]), [b]);
        assert_eq!(
            ids(w, vec![C::MultiEnum { field_id: MULTI as u64, option_ids: vec![MULTI_B as u64], all: false }]),
            Vec::<u64>::new()
        );
        assert!(
            w.search_handle().keyword_candidates("alpha", FilenameStrategy::A, &Filters::default(), 5).unwrap().is_empty(),
            "deleting keyword field is not searchable"
        );
    }
}

#[test]
fn t_umsync_05_renaming_a_field_needs_no_reindex_because_the_index_is_keyed_by_id() {
    // Arrange
    let env = Env::new();
    let [a, ..] = seed(&env);
    let w = env.worker();
    w.drain(&env.conn, &LateClock).unwrap();
    let rev_before: i64 = env.conn.query_row("SELECT revision FROM media_assets WHERE asset_id = ?1", [a as i64], |r| r.get(0)).unwrap();

    // Act: 이름만 변경 (자산 revision·Outbox 를 건드리지 않는다)
    env.conn
        .execute("UPDATE metadata_fields SET name = 'Renamed', name_key = 'renamed', updated_at = 9 WHERE field_id = ?1", [KW])
        .unwrap();

    // Assert
    assert_eq!(w.pending_events(&env.conn).unwrap(), 0);
    let rev_after: i64 = env.conn.query_row("SELECT revision FROM media_assets WHERE asset_id = ?1", [a as i64], |r| r.get(0)).unwrap();
    assert_eq!(rev_before, rev_after);
    assert_eq!(ids(&w, vec![C::KeywordAnyOf { field_id: KW as u64, values: vec!["alpha".into()] }]), [a]);
}

#[test]
fn t_umsync_06_index_built_before_user_fields_is_rebuilt_and_gets_the_values() {
    // Arrange: 사용자 필드가 없던 분석기 버전(v8)으로 만든 인덱스
    let env = Env::new();
    let [a, ..] = seed(&env);
    {
        let w = env.worker();
        w.drain(&env.conn, &LateClock).unwrap();
    }
    std::fs::write(
        env.index_dir().join("mam-search.meta"),
        "format_version=1\nanalyzer_version=8\nbuild_ngram=true\nbuild_raw=true\n",
    )
    .unwrap();

    // Act
    let (w, out) = SyncWorker::open(&env.conn, &env.index_dir(), small(IndexOptions::all()), SyncConfig::default()).unwrap();

    // Assert
    let rebuilt = out.rebuilt.expect("v8 index must be rebuilt");
    assert!(matches!(rebuilt.reason, RebuildReason::Mismatch(_)), "{rebuilt:?}");
    assert_eq!(ids(&w, vec![C::Bool { field_id: BOOL as u64, value: true }]), [a]);
}

#[test]
fn t_umsync_07_value_queries_use_the_primary_key_without_sorting_or_scanning_definitions() {
    // Arrange
    let env = Env::new();
    seed(&env);
    for i in 0..100 {
        let id = add_asset(&env.conn, &format!("f{i}.jpg"));
        edit(&env.conn, id, |t| set(t, id, KW, 0, Val::Text("x")));
    }
    env.conn.execute_batch("ANALYZE").unwrap();

    for (sql, args) in [(VALUES_BY_IDS_SQL, vec!["[1,2,3]"]), (ALL_VALUES_SQL, vec![])] {
        // Act
        let mut stmt = env.conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
        let plan: Vec<String> = stmt
            .query_map(rusqlite::params_from_iter(args), |r| r.get::<_, String>(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();

        // Assert: 정렬 없음, 정의 테이블은 키 검색 (값 테이블 스캔은 전체 스트림에서만)
        let joined = plan.join("\n");
        assert!(!joined.contains("TEMP B-TREE"), "needs a sort: {plan:#?}");
        assert!(!joined.contains("SCAN mf"), "scans definitions per value: {plan:#?}");
        if sql == VALUES_BY_IDS_SQL {
            assert!(!joined.contains("SCAN v"), "{plan:#?}");
        }
    }
    // 빈 목록은 질의하지 않는다
    assert!(load_user_values(&env.conn, &[]).unwrap().is_empty());
}
