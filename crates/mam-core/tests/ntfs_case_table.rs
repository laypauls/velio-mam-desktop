//! T-id-18: NTFS 대소문자 규칙 vs `mam_db::path_key`(Rust 유니코드 소문자화) 불일치 조사 (P1-1 잔여 위험).
//!
//! 분류: Boundary / External
//!
//! 관심 방향: `path_key` 는 같다고 보지만 NTFS 는 다른 파일로 보는 쌍. 이런 쌍은 한 폴더에 공존할 수 있고,
//! 공존하면 D6(스캔 오류)로 처리된다. 반대 방향(NTFS 는 같고 path_key 는 다름)은 같은 파일이 항상 하나의
//! 표기로만 열거되므로 중복 자산을 만들지 않는다.
#![cfg(windows)]

use std::fs;

use mam_db::path_key;
use tempfile::TempDir;

#[test]
fn t_id_18_path_key_case_folding_vs_ntfs() {
    // Arrange: BMP 의 1:1 대소문자 쌍(대문자 U, 소문자 l)마다 파일 `t<U>` 를 만들고 NTFS 가 `t<l>` 을 같은 파일로 보는지 확인
    let dir = TempDir::new().unwrap();
    let mut keys_equal_ntfs_differs: Vec<(char, char)> = Vec::new();
    let mut checked = 0;
    for code in 0x80u32..=0xFFFF {
        let Some(upper) = char::from_u32(code) else {
            continue;
        };
        let mut lower_it = upper.to_lowercase();
        let (Some(lower), None) = (lower_it.next(), lower_it.next()) else {
            continue;
        };
        if lower == upper {
            continue;
        }
        let name_u = format!("t{upper}");
        let name_l = format!("t{lower}");
        if path_key(&name_u).unwrap() != path_key(&name_l).unwrap() {
            continue; // path_key 가 같다고 보는 쌍만 조사
        }
        // Act
        if fs::write(dir.path().join(&name_u), b"x").is_err() {
            continue; // 파일명으로 쓸 수 없는 문자
        }
        checked += 1;
        if !dir.path().join(&name_l).exists() {
            keys_equal_ntfs_differs.push((upper, lower));
        }
    }

    // Assert: 라틴·그리스·키릴 기본 영역(U+0080~U+04FF)은 불일치가 없어야 한다. 예외는 Unicode 7 에서 추가된 U+037F 뿐
    // (NTFS 대문자 표는 볼륨 포맷 시점에 고정돼 신규 문자를 모른다)
    eprintln!(
        "T-id-18: checked {checked} pairs, path_key==same but NTFS differs: {} (first 20: {:?})",
        keys_equal_ntfs_differs.len(),
        keys_equal_ntfs_differs.iter().take(20).collect::<Vec<_>>()
    );
    let common: Vec<_> = keys_equal_ntfs_differs
        .iter()
        .filter(|(u, _)| (*u as u32) <= 0x04FF && *u != '\u{037F}')
        .collect();
    assert!(checked > 500, "sweep too small: {checked}");
    assert!(
        common.is_empty(),
        "Latin/Greek/Cyrillic mismatches: {common:?}"
    );
}
