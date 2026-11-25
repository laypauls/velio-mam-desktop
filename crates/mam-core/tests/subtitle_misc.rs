//! to_srt, 언어 표, 원본 보호.
//! 분류: T-sub-13 Happy / T-sub-14 Business rule / T-sub-16 원본 보호

mod subtitle_support;
use std::collections::BTreeMap;
use std::fs;
use std::time::{Duration, SystemTime};

use mam_core::subtitle::{Cue, load_sidecar_tracks, normalize_language, to_srt};
use subtitle_support::*;

fn cue(s: i64, e: i64, t: &str) -> Cue {
    Cue {
        start_ms: s,
        end_ms: e,
        text: t.to_owned(),
    }
}

#[test]
fn t_sub_13_to_srt_format_and_roundtrip() {
    // Arrange
    let cues = [
        cue(1500, 2000, "a"),
        cue(3_661_001, 3_723_999, "x\ny"),
        cue(360_000_000, 360_001_000, "100h"),
    ];
    // Act
    let srt = to_srt(&cues);
    // Assert
    assert_eq!(
        srt,
        "\u{feff}1\n00:00:01,500 --> 00:00:02,000\na\n\n\
2\n01:01:01,001 --> 01:02:03,999\nx\ny\n\n\
3\n100:00:00,000 --> 100:00:01,000\n100h\n\n"
    );
    assert_eq!(to_srt(&[]), "\u{feff}");
    // 다시 파싱하면 같은 큐 (BOM 포함 UTF-8)
    let d = tmp();
    put(d.path(), "영상.srt", srt.as_bytes());
    assert_eq!(load(d.path(), "영상.mp4")[0].cues, cues);
}

#[test]
fn t_sub_14_language_table_and_unknown_values() {
    // Arrange
    let table: &[(&str, &[&str])] = &[
        ("ko", &["ko", "kor", "ko-kr", "kr", "korean"]),
        ("en", &["en", "eng", "en-us", "en-gb", "english"]),
        ("ja", &["ja", "jpn", "jp", "ja-jp", "japanese"]),
        (
            "zh",
            &[
                "zh", "chi", "zho", "zh-cn", "zh-tw", "zh-hans", "zh-hant", "chs", "cht", "chinese",
            ],
        ),
        ("fr", &["fr", "fre", "fra"]),
        ("de", &["de", "ger", "deu"]),
        ("es", &["es", "spa"]),
        ("it", &["it", "ita"]),
        ("pt", &["pt", "por", "pt-br"]),
        ("ru", &["ru", "rus"]),
        ("vi", &["vi", "vie"]),
        ("th", &["th", "tha"]),
    ];
    // Act + Assert
    for (canon, aliases) in table {
        for a in *aliases {
            assert_eq!(normalize_language(a), Some(*canon), "{a}");
            assert_eq!(
                normalize_language(&a.to_uppercase()),
                Some(*canon),
                "{a} upper"
            );
        }
    }
    assert_eq!(normalize_language("ko_KR"), Some("ko")); // `_` → `-`
    assert_eq!(normalize_language(" Eng "), Some("en"));
    for bad in ["", "final", "sub", "x", "korea", "ko-xx", "v2", "1080p"] {
        assert_eq!(normalize_language(bad), None, "{bad}");
    }
}

#[test]
fn t_sub_16_sources_are_untouched_by_discovery_and_parsing() {
    // Arrange: CP949 SRT·SMI·VTT·ASS + 영상. 읽기 전용 + 과거 mtime 으로 만든다
    let d = tmp();
    let (cp949, _, _) = encoding_rs::EUC_KR.encode(SRT_OK);
    put(d.path(), "영상.srt", &cp949);
    put(
        d.path(),
        "영상.en.smi",
        b"<SAMI><BODY><SYNC Start=1><P Class=ENCC>a</BODY></SAMI>",
    );
    put(
        d.path(),
        "영상.vtt",
        b"WEBVTT\n\n00:01.000 --> 00:02.000\nhi\n",
    );
    put(
        d.path(),
        "영상.ass",
        b"[Events]\nFormat: Start, End, Text\nDialogue: 0:00:01.00,0:00:02.00,hi\n",
    );
    let video = put(d.path(), "영상.mp4", &[7u8; 4096]);
    let old = SystemTime::now() - Duration::from_secs(3600);
    for e in fs::read_dir(d.path()).unwrap() {
        let p = e.unwrap().path();
        fs::OpenOptions::new()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let mut perm = fs::metadata(&p).unwrap().permissions();
        perm.set_readonly(true);
        fs::set_permissions(&p, perm).unwrap();
    }
    let snapshot = || {
        fs::read_dir(d.path())
            .unwrap()
            .map(|e| {
                let p = e.unwrap().path();
                let meta = fs::metadata(&p).unwrap();
                let hash = blake3::hash(&fs::read(&p).unwrap()).to_hex().to_string();
                (
                    p.file_name().unwrap().to_string_lossy().into_owned(),
                    (hash, meta.modified().unwrap(), meta.len()),
                )
            })
            .collect::<BTreeMap<_, _>>()
    };
    let before = snapshot();
    // Act
    let tracks = load_sidecar_tracks(&video);
    let again = load_sidecar_tracks(&video);
    let after = snapshot();
    // Assert: 읽기 전용이어도 모두 로드됨(쓰기 시도 없음), 내용 해시·mtime·크기·파일 목록 불변
    assert_eq!(tracks.len(), 4);
    assert_eq!(tracks, again);
    assert_eq!(before, after);
    assert_eq!(before.len(), 5);
    // Cleanup: 임시 폴더 삭제가 가능하도록 쓰기 권한 복구
    for e in fs::read_dir(d.path()).unwrap() {
        let p = e.unwrap().path();
        let mut perm = fs::metadata(&p).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(&p, perm).unwrap();
    }
}

#[test]
fn t_sub_13b_to_srt_replaces_angle_brackets_but_not_ampersand() {
    // Arrange: 꺾쇠는 마크업으로 해석될 수 있다 (P6, libVLC 실측) - 엔티티는 해석되지 않으므로 & 는 그대로
    let cues = [cue(0, 1000, "A<B & C>D\n<b>x</b> &amp; &lt;")];
    // Act
    let srt = to_srt(&cues);
    // Assert
    assert_eq!(
        srt,
        "\u{feff}1\n00:00:00,000 --> 00:00:01,000\nA\u{2039}B & C\u{203a}D\n\u{2039}b\u{203a}x\u{2039}/b\u{203a} &amp; &lt;\n\n"
    );
    // 원본 큐는 바뀌지 않는다 (치환은 직렬화 시점뿐)
    assert_eq!(cues[0].text, "A<B & C>D\n<b>x</b> &amp; &lt;");
}
