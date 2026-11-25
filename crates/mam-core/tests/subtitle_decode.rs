//! 인코딩 판별 테스트.
//! 분류: T-sub-04 Happy / T-sub-05 Validation

mod subtitle_support;
use subtitle_support::*;

const TEXT: &str = "1\n00:00:01,000 --> 00:00:02,000\n안녕하세요 똠방각하\n\n";

fn utf16(le: bool) -> Vec<u8> {
    let mut out = if le {
        vec![0xFF, 0xFE]
    } else {
        vec![0xFE, 0xFF]
    };
    for u in TEXT.encode_utf16() {
        out.extend(if le { u.to_le_bytes() } else { u.to_be_bytes() });
    }
    out
}

fn cp949() -> Vec<u8> {
    let (bytes, _, unmappable) = encoding_rs::EUC_KR.encode(TEXT);
    assert!(!unmappable);
    assert!(
        std::str::from_utf8(&bytes).is_err(),
        "fixture must not be valid UTF-8"
    );
    bytes.into_owned()
}

#[test]
fn t_sub_04_all_encodings_decode_to_the_same_text() {
    // Arrange
    let mut with_bom = vec![0xEF, 0xBB, 0xBF];
    with_bom.extend(TEXT.as_bytes());
    let variants: Vec<(&str, Vec<u8>)> = vec![
        ("utf8", TEXT.as_bytes().to_vec()),
        ("utf8-bom", with_bom),
        ("utf16le", utf16(true)),
        ("utf16be", utf16(false)),
        ("cp949", cp949()),
    ];
    for (label, bytes) in variants {
        let d = tmp();
        put(d.path(), "영상.srt", &bytes);
        // Act
        let tracks = load(d.path(), "영상.mp4");
        // Assert
        assert_eq!(tracks.len(), 1, "{label}");
        assert_eq!(tracks[0].cues.len(), 1, "{label}");
        assert_eq!(tracks[0].cues[0].text, "안녕하세요 똠방각하", "{label}");
        assert_eq!(tracks[0].cues[0].start_ms, 1000, "{label}");
    }
}

#[test]
fn t_sub_05_undecodable_file_is_excluded_but_others_survive() {
    // Arrange: UTF-8 도 CP949 도 아닌 바이트 (0xFF 는 EUC-KR 선행 바이트가 아니다)
    let mut bad = b"1\n00:00:01,000 --> 00:00:02,000\n".to_vec();
    bad.extend([0xFF, 0xFF, b'\n']);
    let d = tmp();
    put(d.path(), "영상.srt", &bad);
    put(d.path(), "영상.ko.srt", TEXT.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(file_names(&tracks), ["영상.ko.srt"]);
}

#[test]
fn t_sub_05_truncated_utf16_and_nul_binary_are_excluded() {
    // Arrange
    let d = tmp();
    put(d.path(), "영상.srt", &[0xFF, 0xFE, 0x31]); // 홀수 바이트 UTF-16
    put(d.path(), "영상.en.srt", b"\x00\x00\x00\x00\x01"); // NUL 바이너리
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert!(tracks.is_empty());
}
