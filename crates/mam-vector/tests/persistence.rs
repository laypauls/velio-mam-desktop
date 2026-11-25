//! T-vec-07·08: 저장 -> load / view, 손상·메타 불일치.
//! 분류: Happy / External failure / Validation.

mod common;

use common::*;
use mam_vector::AnnIndex;
use mam_vector::{Engine, FlatIndex, OpenMode, Quantization, VectorError, open_index, save_index};

fn same_results(a: &dyn AnnIndex, b: &dyn AnnIndex, d: &[Vec<f32>], l: &str) {
    assert_eq!(a.len(), b.len(), "{l}");
    for probe in [0usize, 11, 99, 250] {
        let ra = a.search(&d[probe], 10, &params(), None).unwrap();
        let rb = b.search(&d[probe], 10, &params(), None).unwrap();
        let ia: Vec<u64> = ra.iter().map(|h| h.id).collect();
        let ib: Vec<u64> = rb.iter().map(|h| h.id).collect();
        // 같은 파일에서 연 인덱스는 같은 그래프 -> 완전 일치해야 한다 (HNSW 재현성 포함)
        assert_eq!(ia, ib, "{l} probe {probe}");
    }
}

// ---------------------------------------------------------------- T-vec-07 (Happy)
#[test]
fn t_vec_07_save_then_load_and_view_match() {
    let d = dataset(300, DIM, 3);
    for s in matrix() {
        let l = label(&s);
        let dir = tempfile::tempdir().unwrap();
        let idx = build(&s, &d);
        save_index(dir.path(), &*idx, &s).unwrap();

        let loaded = open_index(dir.path(), &s, OpenMode::Load).unwrap();
        same_results(&*idx, &*loaded, &d, &l);
        // 불러온 뒤에도 쓸 수 있다
        loaded.add(1_000, &d[5]).unwrap();
        assert_eq!(loaded.len(), 301, "{l}");

        let view = open_index(dir.path(), &s, OpenMode::View).unwrap();
        same_results(&*idx, &*view, &d, &l);
        // view 는 읽기 전용
        assert!(
            matches!(view.add(1_000, &d[5]), Err(VectorError::ReadOnly)),
            "{l}"
        );
        assert!(matches!(view.remove(1), Err(VectorError::ReadOnly)), "{l}");
    }
}

/// Windows 에서 한글 경로(사용자 이름이 한글인 PC)로 save/load/view 가 되는지 — C++ 엔진은 좁은 문자열 경로를 쓴다.
#[test]
fn t_vec_07b_non_ascii_directory() {
    let d = dataset(100, DIM, 4);
    for s in matrix() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("사용자 데이터").join("벡터 인덱스");
        let idx = build(&s, &d);
        save_index(&dir, &*idx, &s).unwrap();
        let loaded = open_index(&dir, &s, OpenMode::Load).unwrap();
        assert_eq!(loaded.len(), 100, "{}", label(&s));
        let view = open_index(&dir, &s, OpenMode::View).unwrap();
        assert_eq!(view.len(), 100, "{}", label(&s));
    }
}

// ---------------------------------------------------------------- T-vec-08 (External failure)
#[test]
fn t_vec_08_truncated_or_corrupt_files_return_error_and_rebuild_works() {
    let d = dataset(300, DIM, 5);
    for s in matrix() {
        let l = label(&s);
        let dir = tempfile::tempdir().unwrap();
        let idx = build(&s, &d);
        save_index(dir.path(), &*idx, &s).unwrap();
        drop(idx);
        let data = s.engine.data_path(dir.path());
        let target = data.clone();
        let original = std::fs::read(&target).unwrap();

        for (name, bytes) in [
            ("truncated half", original[..original.len() / 2].to_vec()),
            ("truncated to 10 bytes", original[..10].to_vec()),
            ("empty", Vec::new()),
        ] {
            std::fs::write(&target, &bytes).unwrap();
            for mode in [OpenMode::Load, OpenMode::View] {
                // 패닉 없이 오류 (테스트 프로세스가 죽지 않는 것이 단언의 일부)
                let r = open_index(dir.path(), &s, mode);
                assert!(
                    matches!(r, Err(VectorError::Corrupt(_)) | Err(VectorError::Io(_))),
                    "{l} {name} {mode:?} -> {:?}",
                    r.err()
                );
            }
        }

        // 삭제 후 재구축 성공
        std::fs::remove_dir_all(dir.path()).unwrap();
        let rebuilt = build(&s, &d);
        save_index(dir.path(), &*rebuilt, &s).unwrap();
        assert_eq!(
            open_index(dir.path(), &s, OpenMode::Load).unwrap().len(),
            300,
            "{l}"
        );
    }
}

#[test]
fn t_vec_08b_meta_mismatch_is_reported_for_each_field() {
    let d = dataset(50, DIM, 6);
    for s in matrix() {
        let l = label(&s);
        let dir = tempfile::tempdir().unwrap();
        save_index(dir.path(), &*build(&s, &d), &s).unwrap();

        let mut wrong_model = s.clone();
        wrong_model.model_id = "other".into();
        let mut wrong_version = s.clone();
        wrong_version.model_version = "2".into();
        let mut wrong_dim = s.clone();
        wrong_dim.dimension = DIM * 2;
        let mut cases = vec![wrong_model, wrong_version, wrong_dim];
        let mut wrong_q = s.clone();
        wrong_q.quantization = match s.quantization {
            Quantization::F32 => Quantization::I8,
            _ => Quantization::F32,
        };
        cases.push(wrong_q);
        for bad in cases {
            let r = open_index(dir.path(), &bad, OpenMode::Load);
            assert!(
                matches!(r, Err(VectorError::IndexMismatch(_))),
                "{l} {:?}",
                r.err()
            );
        }

        // 메타 파일만 지우면 (데이터는 있음) 불일치로 취급
        std::fs::remove_file(dir.path().join("mam-vector.meta")).unwrap();
        assert!(matches!(
            open_index(dir.path(), &s, OpenMode::Load),
            Err(VectorError::IndexMismatch(_))
        ));
    }
}

#[test]
fn t_vec_08c_missing_directory_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let s = spec(Engine::Flat, Quantization::F16);
    let r = open_index(&dir.path().join("none"), &s, OpenMode::Load);
    assert!(matches!(r, Err(VectorError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound));
}

/// flat 파일 헤더가 다른 차원/정밀도를 말하면 (메타를 우회해 직접 열어도) 불일치다.
#[test]
fn t_vec_08d_flat_header_dimension_mismatch() {
    let d = dataset(20, DIM, 8);
    let dir = tempfile::tempdir().unwrap();
    let flat = FlatIndex::new(DIM, Quantization::F16);
    for (i, v) in d.iter().enumerate() {
        flat.add(i as u64 + 1, v).unwrap();
    }
    let path = dir.path().join("x.flat");
    flat.save_file(&path).unwrap();
    for mode in [OpenMode::Load, OpenMode::View] {
        assert!(matches!(
            FlatIndex::open(&path, DIM + 1, Quantization::F16, mode),
            Err(VectorError::IndexMismatch(_))
        ));
        assert!(matches!(
            FlatIndex::open(&path, DIM, Quantization::I8, mode),
            Err(VectorError::IndexMismatch(_))
        ));
    }
}
