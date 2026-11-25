//! P4 복원력 — 벡터 인덱스 손상 재구축(T-res-06), raw 임베딩 꼬리·길이 손상(T-res-07), 모델 교체 재색인(T-res-11).
//! 분류: T-res-06 External / T-res-07 Boundary / T-res-11 Business rule. 실제 SQLite(마이그레이션) +
//! 임시 디렉터리 raw/인덱스. 원본 보호: 시나리오 끝에 픽스처 원본 불변 단언.

mod common;
#[path = "common/resilience.rs"]
mod res;

use std::fs::OpenOptions;
use std::io::Write;

use common::{DIM as CDIM, dataset, naive_topk};
use mam_vector::{AnnIndex, Generations, OpenMode, VectorError, normalized, sync_incremental};
use res::*;

const N: usize = 60;

fn queries(d: &[Vec<f32>]) -> Vec<Vec<f32>> {
    [0usize, 7, 31, 59].iter().map(|i| d[*i].clone()).collect()
}

/// 자산 N 건에 v1 벡터를 저장하고 첫 세대(g1)를 만든 환경.
fn built() -> (Env, mam_vector::ModelKey, Vec<Vec<f32>>) {
    assert_eq!(CDIM, DIM);
    let mut e = env(N as i64);
    let k = key("1");
    let d = dataset(N, DIM, 42);
    put_one_each(&mut e, &k, &d, 1);
    let (idx, rep) = open_or_rebuild(&e, &k);
    assert_eq!(rep.expect("first open builds").vectors, N as u64);
    drop(idx);
    (e, k, d)
}

type Damage = fn(&Env, &mam_vector::ModelKey);

fn damages() -> Vec<(&'static str, Damage)> {
    vec![
        ("data_header_garbage_same_length", |e, k| {
            let f = data_file(&gen_dir(e, k, &active_gen(e, k)));
            let mut b = std::fs::read(&f).unwrap();
            b[..16].iter_mut().for_each(|x| *x = 0xEE);
            std::fs::write(f, b).unwrap();
        }),
        ("data_truncated_to_zero", |e, k| {
            std::fs::write(data_file(&gen_dir(e, k, &active_gen(e, k))), b"").unwrap();
        }),
        ("data_extra_trailing_bytes", |e, k| {
            let f = data_file(&gen_dir(e, k, &active_gen(e, k)));
            OpenOptions::new()
                .append(true)
                .open(f)
                .unwrap()
                .write_all(&[7u8; 13])
                .unwrap();
        }),
        ("data_deleted", |e, k| {
            std::fs::remove_file(data_file(&gen_dir(e, k, &active_gen(e, k)))).unwrap();
        }),
        ("meta_deleted", |e, k| {
            std::fs::remove_file(gen_dir(e, k, &active_gen(e, k)).join("mam-vector.meta")).unwrap();
        }),
        ("meta_dimension_mismatch", |e, k| {
            let p = gen_dir(e, k, &active_gen(e, k)).join("mam-vector.meta");
            let t = std::fs::read_to_string(&p)
                .unwrap()
                .replace(&format!("dimension={DIM}"), "dimension=7");
            std::fs::write(p, t).unwrap();
        }),
        ("meta_model_id_mismatch", |e, k| {
            let p = gen_dir(e, k, &active_gen(e, k)).join("mam-vector.meta");
            let t = std::fs::read_to_string(&p)
                .unwrap()
                .replace("model_id=clip-test", "model_id=other-model");
            std::fs::write(p, t).unwrap();
        }),
        ("meta_non_utf8", |e, k| {
            let p = gen_dir(e, k, &active_gen(e, k)).join("mam-vector.meta");
            std::fs::write(p, b"\x00\xFF\xFE\x80").unwrap();
        }),
        ("active_pointer_to_missing_generation", |e, k| {
            let p = e.store.key_dir(k).join("gen").join("ACTIVE");
            std::fs::write(p, "g99").unwrap();
        }),
        ("active_pointer_empty", |e, k| {
            let p = e.store.key_dir(k).join("gen").join("ACTIVE");
            std::fs::write(p, "").unwrap();
        }),
    ]
}

#[test]
fn t_res_06_damaged_vector_index_is_rebuilt_from_sqlite_and_raw_with_identical_full_ranking() {
    common::affinity::exclude_inconsistent_cores();
    for (name, damage) in damages() {
        // Arrange: 정상 세대의 전수 순위를 기록한 뒤 손상시킨다
        let (e, k, d) = built();
        let originals = Originals::new(2);
        let qs = queries(&d);
        let before = {
            let gens = Generations::new(&e.store, &k).unwrap();
            let idx = gens.open_active(&k, OpenMode::Load).unwrap().unwrap();
            rankings(&idx, &qs)
        };
        let old_gen = active_gen(&e, &k);
        damage(&e, &k);

        // Act: 시작 시 열기 -> 재구축
        let (idx, rep) = open_or_rebuild(&e, &k);

        // Assert: 재구축됐고, 전수 비교 결과(id·점수)가 손상 전과 같다
        let rep = rep.unwrap_or_else(|| panic!("{name}: damage was not detected"));
        assert_eq!(
            (rep.vectors, rep.skipped.is_empty()),
            (N as u64, true),
            "{name}"
        );
        assert_ne!(rep.generation, old_gen, "{name}: new generation");
        assert_eq!(rankings(&idx, &qs), before, "{name}");
        let truth = naive_topk(&d, &d[7], 10);
        assert_eq!(top_assets(&e, &k, &idx, &d[7], 10)[0], truth[0], "{name}");
        // 재구축 뒤 재시작은 저장된 세대를 그대로 연다
        drop(idx);
        let (_idx2, again) = open_or_rebuild(&e, &k);
        assert!(again.is_none(), "{name}: rebuilt twice");
        originals.assert_untouched();
    }
}

#[test]
fn t_res_06b_interrupted_rebuild_leaves_active_generation_usable_and_next_rebuild_cleans_up() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: g1 활성. 재구축 도중 종료되어 meta 없는 반쪽 g2 디렉터리만 남았다
    let (e, k, d) = built();
    let qs = queries(&d);
    let before = {
        let gens = Generations::new(&e.store, &k).unwrap();
        rankings(&gens.open_active(&k, OpenMode::Load).unwrap().unwrap(), &qs)
    };
    let half = gen_dir(&e, &k, "g2");
    std::fs::create_dir_all(&half).unwrap();
    let g1_bytes = std::fs::read(data_file(&gen_dir(&e, &k, "g1"))).unwrap();
    std::fs::write(data_file(&half), &g1_bytes[..g1_bytes.len() / 2]).unwrap();

    // Act 1: 재시작 — 활성 포인터는 그대로 g1 이므로 재구축 없이 쓴다
    let (idx, rep) = open_or_rebuild(&e, &k);

    // Assert 1
    assert!(rep.is_none());
    assert_eq!(active_gen(&e, &k), "g1");
    assert_eq!(rankings(&idx, &qs), before);
    drop(idx);

    // Act 2: 이번엔 g1 이 손상 -> 반쪽 g2 와 충돌하지 않고 g3 로 재구축, 반쪽 세대 정리
    std::fs::write(data_file(&gen_dir(&e, &k, "g1")), b"").unwrap();
    let (idx, rep) = open_or_rebuild(&e, &k);

    // Assert 2
    let rep = rep.expect("rebuilt");
    assert_eq!(rep.generation, "g3");
    assert_eq!(rankings(&idx, &qs), before);
    assert!(!half.exists(), "leftover partial generation is cleaned up");
}

#[test]
fn t_res_06c_bitrot_in_data_region_is_repaired_by_explicit_rebuild() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 길이·헤더는 정상이고 벡터 영역 바이트만 변질 (파일 형식에 체크섬이 없어 열기만으로는 알 수 없다)
    let (e, k, d) = built();
    let qs = queries(&d);
    let gens = Generations::new(&e.store, &k).unwrap();
    let before = rankings(&gens.open_active(&k, OpenMode::Load).unwrap().unwrap(), &qs);
    let f = data_file(&gen_dir(&e, &k, "g1"));
    let mut b = std::fs::read(&f).unwrap();
    let n = b.len();
    b[n - 256..].iter_mut().for_each(|x| *x ^= 0x5A);
    std::fs::write(&f, b).unwrap();

    // Act: 운영자/진단이 명시적으로 재구축
    let rep = gens.rebuild(&e.conn, &e.store, &k).unwrap();

    // Assert: SQLite+raw 에서 다시 만든 세대는 손상 전과 동일
    assert_eq!(rep.vectors, N as u64);
    let idx = gens.open_active(&k, OpenMode::Load).unwrap().unwrap();
    assert_eq!(rankings(&idx, &qs), before);
}

#[test]
fn t_res_07_orphan_raw_tail_is_ignored_and_following_writes_stay_consistent() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 정상 저장 후 raw 꼬리에 (a) 온전한 길이의 고아 레코드 2개 (b) 잘린 레코드 조각이 남았다
    let (mut e, k, d) = built();
    let originals = Originals::new(2);
    let (before_all, skipped0) = read_all(&e, &k);
    assert!(skipped0.is_empty());
    let raw = e.store.raw_path(&k);
    let clean_len = std::fs::metadata(&raw).unwrap().len();
    {
        let mut f = OpenOptions::new().append(true).open(&raw).unwrap();
        f.write_all(&[0x3C; DIM * 2 * 2]).unwrap();
        f.write_all(&[0x11; 13]).unwrap();
    }

    // Act 1: 읽기와 재구축
    let (after_all, skipped) = read_all(&e, &k);
    let gens = Generations::new(&e.store, &k).unwrap();
    let rep = gens.rebuild(&e.conn, &e.store, &k).unwrap();

    // Assert 1: 고아는 무시되고 건너뜀으로도 보고되지 않는다
    assert_eq!(after_all, before_all);
    assert!(skipped.is_empty());
    assert_eq!((rep.vectors, rep.skipped.is_empty()), (N as u64, true));

    // Act 2: 새 벡터 저장 — 잘린 조각은 정리되고 새 레코드는 SQLite 오프셋으로 정확히 읽힌다
    let extra = dataset(1, DIM, 99).remove(0);
    e.store
        .put(&mut e.conn, &k, 3, &[nv(&extra)], NOW + 1)
        .unwrap();
    let (final_all, skipped) = read_all(&e, &k);

    // Assert 2
    assert!(skipped.is_empty());
    assert_eq!(
        final_all.len(),
        N,
        "asset 3 re-embedded: old row superseded"
    );
    let new = final_all
        .iter()
        .find(|(id, _)| *id == (N as u64) + 1)
        .unwrap();
    let want = normalized(&extra);
    assert!(new.1.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-3));
    for (id, v) in &final_all {
        if *id != 3 && *id != (N as u64) + 1 {
            let old = before_all.iter().find(|(i, _)| i == id).unwrap();
            assert_eq!(&old.1, v, "untouched record {id}");
        }
    }
    let len = std::fs::metadata(&raw).unwrap().len();
    assert!(
        len > clean_len && len.is_multiple_of(DIM as u64 * 2),
        "{len}"
    );
    let _ = &d;
    originals.assert_untouched();
}

#[test]
fn t_res_07b_truncated_live_records_are_skipped_reported_and_rest_stay_searchable() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 파일 끝 1.5 레코드가 사라졌다 (마지막 두 활성 레코드 손상, 그 앞은 정상)
    let (e, k, d) = built();
    let originals = Originals::new(2);
    let raw = e.store.raw_path(&k);
    let rl = DIM as u64 * 2;
    let len = std::fs::metadata(&raw).unwrap().len();
    OpenOptions::new()
        .write(true)
        .open(&raw)
        .unwrap()
        .set_len(len - rl - rl / 2)
        .unwrap();
    let gens = Generations::new(&e.store, &k).unwrap();

    // Act
    let rep = gens.rebuild(&e.conn, &e.store, &k).unwrap();
    let (recs, skipped) = read_all(&e, &k);

    // Assert: 손상된 두 건만 건너뛰고 나머지는 정상
    assert_eq!(rep.skipped, vec![N as u64 - 1, N as u64]);
    assert_eq!(skipped, rep.skipped);
    assert_eq!((rep.vectors, recs.len()), (N as u64 - 2, N - 2));
    let idx = gens.open_active(&k, OpenMode::Load).unwrap().unwrap();
    assert_eq!(idx.len(), N - 2);
    let hits = top_assets(&e, &k, &idx, &d[3], 5);
    assert_eq!(hits[0], 4, "asset 4's own vector is the nearest");
    assert!(
        !hits.contains(&(N as u64)),
        "damaged record is never returned"
    );
    originals.assert_untouched();
}

#[test]
fn t_res_07c_missing_raw_file_reports_everything_skipped_without_failing() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange
    let (e, k, _) = built();
    std::fs::remove_file(e.store.raw_path(&k)).unwrap();
    let gens = Generations::new(&e.store, &k).unwrap();

    // Act
    let rep = gens.rebuild(&e.conn, &e.store, &k).unwrap();

    // Assert: SQLite 행은 남아 있고(원천 유지) 읽을 수 없는 id 는 모두 보고된다
    assert_eq!(rep.vectors, 0);
    assert_eq!(rep.skipped, (1..=N as u64).collect::<Vec<_>>());
    assert_eq!(e.store.active_ids(&e.conn, &k).unwrap().len(), N);
}

/// 실제 결함 재현: 파일 꼬리가 잘려 SQLite 가 가리키는 활성 레코드가 파일 밖이 된 뒤 새 벡터를 저장하면
/// `RawFile::open` 이 파일을 레코드 경계로 줄이고 그 자리에 새 레코드를 붙여, 손상된 활성 행(embedding N)의
/// `raw_offset` 이 새 벡터를 가리키게 된다 -> 손상이 `skipped` 로 보고되지 않고 다른 자산의 벡터가 조용히 섞인다.
/// 기대: 손상 레코드는 계속 건너뜀으로 보고되거나, 오프셋이 겹치지 않아야 한다.
#[test]
fn t_res_07d_append_after_truncation_must_not_alias_damaged_live_record() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 마지막 활성 레코드(embedding N, asset N)가 통째로 사라진 raw
    let (mut e, k, _) = built();
    let raw = e.store.raw_path(&k);
    let len = std::fs::metadata(&raw).unwrap().len();
    OpenOptions::new()
        .write(true)
        .open(&raw)
        .unwrap()
        .set_len(len - DIM as u64 * 2)
        .unwrap();
    let (_, skipped) = read_all(&e, &k);
    assert_eq!(
        skipped,
        vec![N as u64],
        "damage is detected before the write"
    );

    // Act: 다른 자산(1)의 새 벡터 저장
    let fresh = dataset(1, DIM, 7).remove(0);
    e.store
        .put(&mut e.conn, &k, 1, &[nv(&fresh)], NOW + 1)
        .unwrap();
    let (recs, skipped) = read_all(&e, &k);

    // Assert: 손상된 레코드(embedding N)가 새 벡터를 대신 읽어서는 안 된다
    let aliased = recs
        .iter()
        .filter(|(id, _)| *id == N as u64 || *id == N as u64 + 1)
        .count();
    assert!(
        skipped.contains(&(N as u64)) || aliased == 1,
        "damaged embedding {N} silently reads the new record: skipped={skipped:?}"
    );
}

#[test]
fn t_res_11_model_swap_reembeds_into_a_new_generation_without_mixing_old_vectors() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: v1 로 전부 임베딩·색인(활성). 새 모델 v2 는 다른 벡터 공간이다.
    let (mut e, k1, d1) = built();
    let originals = Originals::new(2);
    let k2 = key("2");
    let d2 = dataset(N, DIM, 4242);
    let v1_ids = e.store.active_ids(&e.conn, &k1).unwrap();
    let q_old = d1[11].clone();
    let q_new = d2[11].clone();

    // Act 1: 재임베딩 도중(일부만) -> 새 모델 인덱스는 이미 만든 것만 담고 옛 벡터로 채우지 않는다
    put_one_each(&mut e, &k2, &d2[..20], 1);
    let g2 = Generations::new(&e.store, &k2).unwrap();
    let (idx2, rep) = g2
        .open_or_rebuild(&e.conn, &e.store, &k2, OpenMode::Load)
        .unwrap();

    // Assert 1
    assert_eq!(rep.expect("new model, new generation").vectors, 20);
    assert_eq!(idx2.len(), 20);
    assert_ne!(e.store.key_dir(&k1), e.store.key_dir(&k2));
    let partial = top_assets(&e, &k2, &idx2, &q_new, 30);
    assert_eq!(partial.len(), 20);
    assert!(partial.iter().all(|a| (1..=20).contains(a)));

    // Act 2: 나머지 재임베딩 + 증분 반영 + 재시작
    put_one_each(&mut e, &k2, &d2[20..], 21);
    let added = sync_incremental(&e.conn, &e.store, &k2, &idx2).unwrap();
    g2.persist(&k2, &idx2).unwrap();
    drop(idx2);
    let (idx2, again) = g2
        .open_or_rebuild(&e.conn, &e.store, &k2, OpenMode::Load)
        .unwrap();

    // Assert 2: 새 세대만 검색 — 결과가 새 모델 벡터의 정확 검색과 일치, 옛 embedding_id 가 없다
    assert_eq!((added.read, added.skipped.len()), (40, 0));
    assert!(again.is_none(), "persisted generation is reused");
    assert_eq!(idx2.len(), N);
    let new_ids = e.store.active_ids(&e.conn, &k2).unwrap();
    assert!(
        new_ids.iter().all(|id| !v1_ids.contains(id)),
        "id spaces are disjoint"
    );
    for qi in [11usize, 30, 55] {
        let got = top_assets(&e, &k2, &idx2, &d2[qi], 10);
        let truth = naive_topk(&d2, &d2[qi], 10);
        assert_eq!(got[0], truth[0], "top-1 q{qi}");
        assert!(common::recall(&got, &truth) >= 0.8, "q{qi}");
    }
    let ranking_ids: Vec<u64> = full_ranking(&idx2, &q_new)
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert!(ranking_ids.iter().all(|id| new_ids.contains(id)));
    // 같은 질의를 옛 모델 질의 벡터로 던져도 새 인덱스에서 옛 embedding 이 나오지 않는다
    let old_q_ranking: Vec<u64> = full_ranking(&idx2, &q_old)
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert!(old_q_ranking.iter().all(|id| !v1_ids.contains(id)));
    // 옛 세대는 교체(폐기) 전까지 그대로 — 교체 후에는 옛 모델 벡터를 비워도 새 검색에 영향이 없다
    let g1 = Generations::new(&e.store, &k1).unwrap();
    assert_eq!(
        g1.open_active(&k1, OpenMode::View).unwrap().unwrap().len(),
        N
    );
    for a in 1..=N as i64 {
        e.store.put(&mut e.conn, &k1, a, &[], NOW + 5).unwrap();
    }
    assert!(e.store.active_ids(&e.conn, &k1).unwrap().is_empty());
    assert_eq!(top_assets(&e, &k2, &idx2, &d2[30], 3)[0], 31);
    originals.assert_untouched();
}

#[test]
fn t_res_11b_stale_index_files_from_old_model_are_rejected_and_rebuilt_from_new_model_only() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 운영 실수/반쯤 끝난 교체로 새 모델 키 디렉터리에 옛 모델의 세대 파일이 복사돼 활성이 됐다
    let (mut e, k1, _) = built();
    let k2 = key("2");
    let d2 = dataset(N, DIM, 4243);
    put_one_each(&mut e, &k2, &d2, 1);
    let dst = gen_dir(&e, &k2, "g1");
    std::fs::create_dir_all(&dst).unwrap();
    for f in ["index.flat", "mam-vector.meta"] {
        std::fs::copy(gen_dir(&e, &k1, "g1").join(f), dst.join(f)).unwrap();
    }
    std::fs::write(e.store.key_dir(&k2).join("gen").join("ACTIVE"), "g1").unwrap();
    let g2 = Generations::new(&e.store, &k2).unwrap();

    // Act
    let opened = g2.open_active(&k2, OpenMode::Load);
    let (idx, rep) = g2
        .open_or_rebuild(&e.conn, &e.store, &k2, OpenMode::Load)
        .unwrap();

    // Assert: 모델 버전 불일치로 거부되고, 새 모델의 벡터만으로 재구축된다
    assert!(matches!(opened.err(), Some(VectorError::IndexMismatch(_))));
    assert_eq!(rep.expect("rebuilt").vectors, N as u64);
    let new_ids = e.store.active_ids(&e.conn, &k2).unwrap();
    let hits = idx.search(&d2[5], 5, &Default::default(), None).unwrap();
    assert!(hits.iter().all(|h| new_ids.contains(&h.id)));
    assert_eq!(top_assets(&e, &k2, &idx, &d2[5], 1), vec![6]);
}

#[test]
fn t_res_11c_new_model_with_different_dimension_is_isolated_and_rejects_old_dimension_vectors() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 차원이 다른 새 모델 (16-d)
    let (mut e, k1, d1) = built();
    let k16 = key_dim("2", 16);

    // Act: 옛 차원(32-d) 벡터를 새 키로 저장 시도
    let rejected = e.store.put(&mut e.conn, &k16, 1, &[nv(&d1[0])], NOW);
    let d16 = dataset(10, 16, 5);
    put_one_each(&mut e, &k16, &d16, 1);
    let g16 = Generations::new(&e.store, &k16).unwrap();
    let (idx16, rep) = g16
        .open_or_rebuild(&e.conn, &e.store, &k16, OpenMode::Load)
        .unwrap();

    // Assert: 거부(파일·DB 무변경), 별도 디렉터리, 옛 모델 인덱스 불변
    assert!(
        matches!(rejected, Err(VectorError::DimensionMismatch { .. })),
        "{rejected:?}"
    );
    assert_eq!((rep.unwrap().vectors, idx16.len()), (10, 10));
    assert_ne!(e.store.key_dir(&k1), e.store.key_dir(&k16));
    assert_eq!(e.store.active_ids(&e.conn, &k1).unwrap().len(), N);
    let g1 = Generations::new(&e.store, &k1).unwrap();
    assert_eq!(
        g1.open_active(&k1, OpenMode::Load).unwrap().unwrap().len(),
        N
    );
}

/// 실제 결함 재현: `gen/ACTIVE` 포인터 파일이 UTF-8 이 아닌 바이트로 손상되면 `open_or_rebuild` 가 `Io` 로 재구축에
/// 들어가지만, `rebuild` -> `IndexRoot::activate` 가 `self.active()?` (swap.rs:70) 에서 같은 `Io(InvalidData)` 로 실패해
/// 재구축이 영구히 실패한다 (새 세대 디렉터리만 쌓이고 활성화되지 못함). 기대: 읽을 수 없는 포인터는 "활성 없음" 으로 보고 재구축.
#[test]
fn t_res_06d_non_utf8_active_pointer_is_rebuilt_not_a_permanent_failure() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange
    let (e, k, d) = built();
    let qs = queries(&d);
    let before = {
        let gens = Generations::new(&e.store, &k).unwrap();
        rankings(&gens.open_active(&k, OpenMode::Load).unwrap().unwrap(), &qs)
    };
    std::fs::write(
        e.store.key_dir(&k).join("gen").join("ACTIVE"),
        b"\xFF\xFE\x00",
    )
    .unwrap();
    let gens = Generations::new(&e.store, &k).unwrap();

    // Act
    let r = gens.open_or_rebuild(&e.conn, &e.store, &k, OpenMode::Load);

    // Assert
    let (idx, rep) = r
        .map_err(|e| e.to_string())
        .expect("pointer corruption must be recoverable");
    assert!(rep.is_some());
    assert_eq!(rankings(&idx, &qs), before);
}

fn gen_dirs(e: &Env, k: &mam_vector::ModelKey) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(e.store.key_dir(k).join("gen"))
        .unwrap()
        .flatten()
        .filter(|d| d.path().is_dir())
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// 회귀(06d): 손상 포인터 재구축 뒤 세대 디렉터리는 활성 1개만 남고, 포인터는 읽을 수 있는 새 세대를 가리킨다.
/// 경로 구분자가 든 포인터도 동일하게 복구된다.
#[test]
fn t_res_06e_corrupt_pointer_rebuild_leaves_single_generation_and_valid_pointer() {
    common::affinity::exclude_inconsistent_cores();
    for bad in [&b"\xFF\xFE\x00"[..], &b"..\\..\\x"[..], &b"../g1"[..]] {
        // Arrange
        let (e, k, _) = built();
        let ptr = e.store.key_dir(&k).join("gen").join("ACTIVE");
        std::fs::write(&ptr, bad).unwrap();
        let gens = Generations::new(&e.store, &k).unwrap();

        // Act
        let (idx, rep) = gens
            .open_or_rebuild(&e.conn, &e.store, &k, OpenMode::Load)
            .expect("recoverable");
        drop(idx);

        // Assert
        let name = rep.expect("rebuilt").generation;
        assert_eq!(gens.active_name().unwrap().as_deref(), Some(name.as_str()));
        assert_eq!(gen_dirs(&e, &k), vec![name]);
        assert!(gens.open_active(&k, OpenMode::Load).unwrap().is_some());
    }
}

/// 회귀(06d): 활성화 실패(메타 없음) 시 방금 만든 세대 디렉터리가 남지 않는다.
#[test]
fn t_res_06f_failed_activation_removes_generation_built_in_that_attempt() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 구축 후 save_index 가 만든 메타를 지울 수는 없으므로, 활성 포인터를 디렉터리(읽기 불가)로 만들어 실패시킨다
    let (e, k, _) = built();
    let gdir = e.store.key_dir(&k).join("gen");
    std::fs::remove_file(gdir.join("ACTIVE")).unwrap();
    std::fs::create_dir(gdir.join("ACTIVE")).unwrap(); // 포인터 자리가 디렉터리 -> rename 실패
    let gens = Generations::new(&e.store, &k).unwrap();
    let before = gen_dirs(&e, &k);

    // Act
    let r1 = gens.rebuild(&e.conn, &e.store, &k);
    let r2 = gens.rebuild(&e.conn, &e.store, &k);

    // Assert
    assert!(r1.is_err() && r2.is_err());
    let after: Vec<_> = gen_dirs(&e, &k)
        .into_iter()
        .filter(|n| n != "ACTIVE")
        .collect();
    assert_eq!(
        after,
        before
            .into_iter()
            .filter(|n| n != "ACTIVE")
            .collect::<Vec<_>>()
    );
}

/// 회귀(07d): 레코드 중간에서 끊긴 꼬리 + 여러 번 put 해도 손상 행은 계속 skipped, 새 행은 자기 벡터를 읽고,
/// 재시작(새 EmbedStore 열기) 뒤에도 같다.
#[test]
fn t_res_07e_partial_tail_then_repeated_puts_keep_damaged_row_skipped_and_new_rows_correct() {
    common::affinity::exclude_inconsistent_cores();
    // Arrange: 마지막 레코드를 절반만 남김
    let (mut e, k, _) = built();
    let raw = e.store.raw_path(&k);
    let len = std::fs::metadata(&raw).unwrap().len();
    OpenOptions::new()
        .write(true)
        .open(&raw)
        .unwrap()
        .set_len(len - DIM as u64)
        .unwrap();

    // Act
    let fresh = dataset(3, DIM, 9);
    for (i, f) in fresh.iter().enumerate() {
        e.store
            .put(&mut e.conn, &k, 1 + i as i64, &[nv(f)], NOW + 1 + i as i64)
            .unwrap();
    }
    let (recs, skipped) = read_all(&e, &k);

    // Assert
    assert_eq!(skipped, vec![N as u64]);
    for (i, f) in fresh.iter().enumerate() {
        let id = N as u64 + 1 + i as u64;
        let (_, got) = recs
            .iter()
            .find(|(r, _)| *r == id)
            .expect("new row readable");
        let want = normalized(f);
        assert!(
            got.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-2),
            "embedding {id} reads its own vector"
        );
    }
}
