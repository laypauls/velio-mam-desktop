//! T-put-01~04: `EmbedStore::put` 의 raw 참조 하한(floor) 캐시 — O(n) `MAX(raw_offset)` 를 키·프로세스당 한 번으로.
//! 분류: Happy / Boundary / Business rule(리셋 뒤 재사용) / Concurrency(인스턴스 여럿).
//! 손상 복구(꼬리 잘림 뒤 겹침 방지)는 `resilience_index.rs` 의 T-res-07d·07e 가 같은 코드 경로로 검증한다.

mod common;

use common::{DIM, dataset};
use mam_vector::{EmbedStore, ModelKey, NewVector, VectorKind, VectorSource};
use rusqlite::Connection;
use tempfile::TempDir;

const REC: u64 = DIM as u64 * 2;

struct Env {
    dir: TempDir,
    conn: Connection,
    store: EmbedStore,
    key: ModelKey,
}

fn env() -> Env {
    let dir = TempDir::new().unwrap();
    let mut conn = mam_db::open(&dir.path().join("t.db")).unwrap();
    mam_db::migrate(&mut conn, &dir.path().join("backup"), &mam_db::SystemClock).unwrap();
    for a in 1..=5 {
        conn.execute(
            "INSERT INTO media_assets (asset_id, media_type, file_size, modified_at, created_at, updated_at)
             VALUES (?1, 'image', 1, 1, 1, 1)",
            [a],
        )
        .unwrap();
    }
    let store = EmbedStore::new(dir.path().join("vectors")).unwrap();
    let key = ModelKey {
        kind: VectorKind::Visual,
        model_id: "m".into(),
        model_version: "1".into(),
        dimension: DIM,
    };
    Env {
        dir,
        conn,
        store,
        key,
    }
}

fn put(store: &EmbedStore, conn: &mut Connection, key: &ModelKey, asset: i64, v: &[Vec<f32>]) {
    let nvs: Vec<NewVector> = v
        .iter()
        .map(|x| NewVector {
            segment_id: None,
            source: VectorSource::Frame,
            frame_index: None,
            vector: x,
        })
        .collect();
    store.put(conn, key, asset, &nvs, 1).unwrap();
}

fn offsets(conn: &Connection) -> Vec<u64> {
    let mut st = conn
        .prepare("SELECT raw_offset FROM embeddings ORDER BY embedding_id")
        .unwrap();
    st.query_map([], |r| r.get::<_, i64>(0))
        .unwrap()
        .map(|o| o.unwrap() as u64)
        .collect()
}

fn raw_len(e: &Env) -> u64 {
    std::fs::metadata(e.store.raw_path(&e.key)).unwrap().len()
}

// ---------------------------------------------------------------- T-put-01 (Happy / Boundary)
#[test]
fn t_put_01_offsets_stay_contiguous_across_many_puts_and_a_restart() {
    // Arrange
    let mut e = env();
    let d = dataset(12, DIM, 1);
    // Act: 다중 프레임·재임베딩·폐기만(빈 벡터) 섞어 저장 -> 재시작(새 EmbedStore) -> 더 저장
    put(&e.store, &mut e.conn, &e.key, 1, &d[0..3]);
    put(&e.store, &mut e.conn, &e.key, 2, &d[3..4]);
    put(&e.store, &mut e.conn, &e.key, 1, &d[4..6]);
    put(&e.store, &mut e.conn, &e.key, 2, &[]);
    let store2 = EmbedStore::new(e.dir.path().join("vectors")).unwrap();
    put(&store2, &mut e.conn, &e.key, 3, &d[6..9]);
    put(&e.store, &mut e.conn, &e.key, 4, &d[9..12]);
    // Assert: 오프셋이 0, REC, 2*REC ... 빈틈·겹침 없이 이어지고 파일 길이와 같다 (인스턴스가 둘이어도)
    let got = offsets(&e.conn);
    assert_eq!(
        got,
        (0..got.len() as u64).map(|i| i * REC).collect::<Vec<_>>()
    );
    assert_eq!(raw_len(&e), got.len() as u64 * REC);
    assert_eq!(got.len(), 12);
}

// ---------------------------------------------------------------- T-put-02 (Business rule: 리셋)
#[test]
fn t_put_02_after_data_reset_the_stale_floor_does_not_pad_the_new_file() {
    // Arrange: 저장 3건 -> 데이터 초기화(DB 행 삭제 + raw 파일 삭제) — 같은 EmbedStore 인스턴스가 살아 있다
    let mut e = env();
    let d = dataset(5, DIM, 2);
    put(&e.store, &mut e.conn, &e.key, 1, &d[0..3]);
    e.conn.execute("DELETE FROM embeddings", []).unwrap();
    std::fs::remove_file(e.store.raw_path(&e.key)).unwrap();
    // Act
    put(&e.store, &mut e.conn, &e.key, 2, &d[3..4]);
    // Assert: 옛 floor(3 레코드)로 HOLE 을 채우지 않고 처음부터 쓴다
    assert_eq!(offsets(&e.conn), vec![0]);
    assert_eq!(raw_len(&e), REC);
}

#[test]
fn t_put_02_rows_deleted_but_file_kept_appends_at_file_end() {
    let mut e = env();
    let d = dataset(5, DIM, 3);
    put(&e.store, &mut e.conn, &e.key, 1, &d[0..2]);
    e.conn.execute("DELETE FROM embeddings", []).unwrap();
    put(&e.store, &mut e.conn, &e.key, 2, &d[2..3]);
    assert_eq!(offsets(&e.conn), vec![2 * REC]);
    assert_eq!(raw_len(&e), 3 * REC);
}

// ---------------------------------------------------------------- T-put-03 (Validation: 검증 실패는 floor 를 건드리지 않는다)
#[test]
fn t_put_03_rejected_vector_leaves_file_and_following_offsets_untouched() {
    let mut e = env();
    let d = dataset(3, DIM, 4);
    put(&e.store, &mut e.conn, &e.key, 1, &d[0..1]);
    let bad = [vec![0.0f32; DIM]]; // 영벡터
    let nvs = [NewVector {
        segment_id: None,
        source: VectorSource::Frame,
        frame_index: None,
        vector: &bad[0],
    }];
    assert!(e.store.put(&mut e.conn, &e.key, 2, &nvs, 1).is_err());
    put(&e.store, &mut e.conn, &e.key, 2, &d[1..2]);
    assert_eq!(offsets(&e.conn), vec![0, REC]);
    assert_eq!(raw_len(&e), 2 * REC);
}
