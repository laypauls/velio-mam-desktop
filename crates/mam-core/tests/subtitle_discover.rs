//! 자막 탐색 테스트.
//! 분류: T-sub-01 Happy / 02 Validation / 03 Boundary / 11 Boundary / 12 Business rule / 15 External

mod subtitle_support;
use std::fs;
use std::path::Path;

use mam_core::subtitle::{MAX_CANDIDATES, MAX_SUBTITLE_BYTES, load_sidecar_tracks};
use subtitle_support::*;
use unicode_normalization::UnicodeNormalization;

const SMI_EN: &str = "<SAMI><BODY><SYNC Start=1000><P Class=ENCC>Hello</BODY></SAMI>";
const VTT_OK: &str = "WEBVTT\n\n00:01.000 --> 00:02.000\nhi\n";
const ASS_OK: &str = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,hi\n";

#[test]
fn t_sub_01_discovers_all_supported_extensions_case_insensitively() {
    // Arrange
    let d = tmp();
    put(d.path(), "영상.srt", SRT_OK.as_bytes());
    put(d.path(), "영상.KO.SRT", SRT_OK.as_bytes());
    put(d.path(), "영상.eng.smi", SMI_EN.as_bytes());
    put(d.path(), "영상.vtt", VTT_OK.as_bytes());
    put(d.path(), "영상.ass", ASS_OK.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(
        sorted(file_names(&tracks)),
        sorted(
            [
                "영상.srt",
                "영상.KO.SRT",
                "영상.eng.smi",
                "영상.vtt",
                "영상.ass"
            ]
            .map(String::from)
            .to_vec()
        )
    );
    let lang = |f: &str| {
        tracks
            .iter()
            .find(|t| t.track.file_name == f)
            .unwrap()
            .track
            .language
            .clone()
    };
    assert_eq!(lang("영상.KO.SRT").as_deref(), Some("ko"));
    assert_eq!(lang("영상.eng.smi").as_deref(), Some("en"));
    assert_eq!(lang("영상.srt"), None);
}

#[test]
fn t_sub_01_stem_match_ignores_case() {
    // Arrange
    let d = tmp();
    put(d.path(), "MOVIE.SRT", SRT_OK.as_bytes());
    // Act
    let tracks = load(d.path(), "Movie.mp4");
    // Assert
    assert_eq!(file_names(&tracks), ["MOVIE.SRT"]);
}

#[test]
fn t_sub_02_rejects_non_matching_names_extensions_and_folders() {
    // Arrange
    let d = tmp();
    put(d.path(), "영상.final.srt", SRT_OK.as_bytes()); // 접미사가 언어 코드가 아님
    put(d.path(), "영상2.srt", SRT_OK.as_bytes());
    put(d.path(), "다른영상.srt", SRT_OK.as_bytes());
    put(d.path(), "영상.srt.bak", SRT_OK.as_bytes());
    put(d.path(), "영상.sub", SRT_OK.as_bytes());
    put(d.path(), "영상.idx", SRT_OK.as_bytes());
    fs::create_dir(d.path().join("영상.ko.srt")).unwrap(); // 폴더
    put(d.path(), "영상.en.srt", SRT_OK.as_bytes()); // 유일한 정상 후보
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(file_names(&tracks), ["영상.en.srt"]);
}

#[test]
fn t_sub_03_matches_nfd_subtitle_to_nfc_video_and_back() {
    // Arrange: 영상은 NFC, 자막 이름은 NFD (macOS 에서 만든 NAS 파일)
    let nfc: String = "한글영상".nfc().collect();
    let nfd: String = "한글영상".nfd().collect();
    assert_ne!(nfc, nfd);
    let d = tmp();
    let sub_name = format!("{nfd}.ko.srt");
    put(d.path(), &sub_name, SRT_OK.as_bytes());
    // Act
    let tracks = load(d.path(), &format!("{nfc}.mp4"));
    // Assert
    assert_eq!(file_names(&tracks), [sub_name]);
    // 반대 방향: 영상 NFD, 자막 NFC
    let d2 = tmp();
    let sub2 = format!("{nfc}.srt");
    put(d2.path(), &sub2, SRT_OK.as_bytes());
    let tracks2 = load(d2.path(), &format!("{nfd}.mp4"));
    assert_eq!(file_names(&tracks2), [sub2]);
}

#[test]
fn t_sub_03_dotted_stem_with_language_suffix() {
    // Arrange
    let d = tmp();
    put(d.path(), "a.b.ko.srt", SRT_OK.as_bytes());
    put(d.path(), "a.ko.srt", SRT_OK.as_bytes()); // 다른 영상(a)의 자막 — 제외
    // Act
    let tracks = load(d.path(), "a.b.mp4");
    // Assert
    assert_eq!(file_names(&tracks), ["a.b.ko.srt"]);
    assert_eq!(tracks[0].track.language.as_deref(), Some("ko"));
}

#[test]
fn t_sub_11_empty_cueless_oversize_files_are_excluded() {
    // Arrange
    let d = tmp();
    put(d.path(), "영상.srt", b""); // 빈 파일
    put(d.path(), "영상.ko.srt", b"no cues here\n"); // 큐 0
    let big = put(d.path(), "영상.en.srt", SRT_OK.as_bytes());
    let f = fs::OpenOptions::new().write(true).open(&big).unwrap();
    f.set_len(MAX_SUBTITLE_BYTES + 1).unwrap(); // 상한 초과
    drop(f);
    put(d.path(), "영상.ja.srt", SRT_OK.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(file_names(&tracks), ["영상.ja.srt"]);
}

#[test]
fn t_sub_11_candidates_beyond_limit_are_dropped_deterministically() {
    // Arrange: 유효한 언어 접미사 33개
    let suffixes = [
        "ko", "kor", "ko-kr", "kr", "korean", "en", "eng", "en-us", "en-gb", "english", "ja",
        "jpn", "jp", "ja-jp", "japanese", "zh", "chi", "zho", "zh-cn", "zh-tw", "zh-hans",
        "zh-hant", "chs", "cht", "chinese", "fr", "fre", "fra", "de", "ger", "deu", "es", "spa",
    ];
    assert_eq!(suffixes.len(), MAX_CANDIDATES + 1);
    let d = tmp();
    for s in suffixes {
        put(d.path(), &format!("영상.{s}.srt"), SRT_OK.as_bytes());
    }
    let mut expected: Vec<String> = suffixes.iter().map(|s| format!("영상.{s}.srt")).collect();
    expected.sort();
    expected.pop(); // 정렬상 마지막 하나가 제외된다
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(tracks.len(), MAX_CANDIDATES);
    assert_eq!(sorted(file_names(&tracks)), expected);
}

#[test]
fn t_sub_12_order_and_ids_are_stable_across_calls() {
    // Arrange
    let d = tmp();
    put(d.path(), "영상.ass", ASS_OK.as_bytes());
    put(d.path(), "영상.vtt", VTT_OK.as_bytes());
    put(d.path(), "영상.srt", SRT_OK.as_bytes());
    put(
        d.path(),
        "영상.ko.smi",
        "<SAMI><BODY><SYNC Start=1><P Class=KRCC>가</BODY></SAMI>".as_bytes(),
    );
    put(d.path(), "영상.ko.srt", SRT_OK.as_bytes());
    put(d.path(), "영상.eng.srt", SRT_OK.as_bytes());
    let video = put(d.path(), "영상.mp4", b"V");
    // Act
    let first = load_sidecar_tracks(&video);
    let second = load_sidecar_tracks(&video);
    // Assert: 언어 오름차순(en, ko), 같은 언어는 srt → smi, 언어 없음은 srt → vtt → ass
    assert_eq!(
        ids(&first),
        [
            "영상.eng.srt",
            "영상.ko.srt",
            "영상.ko.smi#KRCC",
            "영상.srt",
            "영상.vtt",
            "영상.ass"
        ]
    );
    assert_eq!(first, second);
}

#[test]
fn t_sub_15_missing_folder_or_bad_parent_returns_empty_without_panic() {
    // Arrange
    let d = tmp();
    let file = put(d.path(), "plain.txt", b"x");
    // Act + Assert
    assert!(load_sidecar_tracks(&d.path().join("없는폴더").join("영상.mp4")).is_empty());
    assert!(load_sidecar_tracks(&file.join("영상.mp4")).is_empty()); // 부모가 파일
    assert!(load_sidecar_tracks(Path::new("")).is_empty()); // 이름 없음
}
