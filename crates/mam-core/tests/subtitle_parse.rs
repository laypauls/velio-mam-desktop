//! 형식별 파서 테스트.
//! 분류: T-sub-06 Happy / 07 Business rule / 08 Validation / 09 Happy+Validation / 10 Happy / 11 Boundary(큐 상한)

mod subtitle_support;
use mam_core::subtitle::{Cue, MAX_CUES, ParsedTrack};
use subtitle_support::*;

fn cue(s: i64, e: i64, t: &str) -> Cue {
    Cue {
        start_ms: s,
        end_ms: e,
        text: t.to_owned(),
    }
}

fn by_id<'a>(tracks: &'a [ParsedTrack], id: &str) -> &'a ParsedTrack {
    tracks
        .iter()
        .find(|t| t.track.id == id)
        .unwrap_or_else(|| panic!("no track {id}"))
}

#[test]
fn t_sub_06_sami_splits_classes_with_same_class_end_times() {
    // Arrange
    let smi = "<SAMI><HEAD><STYLE TYPE=\"text/css\"><!--\nP { margin-left:8pt; }\n\
.KRCC { Name: 한국어; lang: ko-KR; }\n.ENCC { Name: English; lang: en-US; }\n--></STYLE></HEAD><BODY>\n\
<SYNC Start=1000><P Class=KRCC>안녕<br>하세요<P Class=ENCC>Hello\n\
<SYNC Start=3000><P Class=KRCC>&nbsp;\n\
<SYNC Start=4000><P Class=ENCC>&nbsp;<P Class=KRCC>두번째\n\
<sync start=6000><p class=ENCC>Second &amp; last\n</BODY></SAMI>";
    let d = tmp();
    put(d.path(), "영상.smi", smi.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(ids(&tracks), ["영상.smi#ENCC", "영상.smi#KRCC"]); // en < ko
    let en = by_id(&tracks, "영상.smi#ENCC");
    assert_eq!(en.track.language.as_deref(), Some("en"));
    assert_eq!(
        en.cues,
        [cue(1000, 4000, "Hello"), cue(6000, 11000, "Second & last")]
    );
    let ko = by_id(&tracks, "영상.smi#KRCC");
    assert_eq!(ko.track.language.as_deref(), Some("ko"));
    // KRCC 의 3000 SYNC 는 지움(&nbsp;) → 첫 큐는 3000 에서 끝나고, 마지막 큐는 +5 s
    assert_eq!(
        ko.cues,
        [cue(1000, 3000, "안녕\n하세요"), cue(4000, 9000, "두번째")]
    );
}

#[test]
fn t_sub_07_sami_unknown_class_language_fallback_chain() {
    // Arrange: CC1 → STYLE lang, XX → STYLE Name, ZZ → 파일명 접미사
    let smi = "<SAMI><HEAD><STYLE><!--\n.CC1 { lang: ko-KR; }\n.XX { Name: Japanese; }\n--></STYLE></HEAD><BODY>\
<SYNC Start=1000><P Class=CC1>a<P Class=XX>b<P Class=ZZ>c</BODY></SAMI>";
    let d = tmp();
    put(d.path(), "영상.en.smi", smi.as_bytes());
    let d2 = tmp();
    put(d2.path(), "영상.smi", smi.as_bytes());
    // Act
    let with_suffix = load(d.path(), "영상.mp4");
    let without = load(d2.path(), "영상.mp4");
    // Assert
    let lang = |t: &[ParsedTrack], c: &str| {
        by_id(t, &format!("{}#{c}", t[0].track.file_name))
            .track
            .language
            .clone()
    };
    assert_eq!(lang(&with_suffix, "CC1").as_deref(), Some("ko"));
    assert_eq!(lang(&with_suffix, "XX").as_deref(), Some("ja"));
    assert_eq!(lang(&with_suffix, "ZZ").as_deref(), Some("en"));
    assert_eq!(lang(&without, "ZZ"), None); // 접미사도 없으면 None
    assert_eq!(without.len(), 3);
}

#[test]
fn t_sub_08_srt_skips_only_broken_blocks() {
    // Arrange
    let srt = "1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\n\r\n\
garbage block\r\nno timecode\r\n\r\n\
00:00:03,000 --> 00:00:04,000\r\n번호 없음\r\n\r\n\
4\r\n00:00:05,000 --> 00:00:05,000\r\n길이 0\r\n\r\n\
5\r\n00:00:07,000 --> bad\r\nbroken time\r\n\r\n\
6\r\n00:01:00,500 --> 00:01:02,000\r\n<i>Multi</i>\r\nline\r\n\r\n\
7\r\n00:00:09,000 --> 00:00:08,000\r\nend before start\r\n";
    let d = tmp();
    put(d.path(), "영상.srt", srt.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(tracks.len(), 1);
    assert_eq!(
        tracks[0].cues,
        [
            cue(1000, 2500, "Hello"),
            cue(3000, 4000, "번호 없음"),
            cue(60500, 62000, "Multi\nline")
        ]
    );
}

#[test]
fn t_sub_08_srt_sorts_by_start_and_allows_overlap() {
    // Arrange
    let srt = "1\n00:00:05,000 --> 00:00:09,000\nlate\n\n2\n00:00:01,000 --> 00:00:06,000\nearly\n";
    let d = tmp();
    put(d.path(), "영상.srt", srt.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(
        tracks[0].cues,
        [cue(1000, 6000, "early"), cue(5000, 9000, "late")]
    );
}

#[test]
fn t_sub_09_vtt_skips_note_style_and_strips_tags() {
    // Arrange
    let vtt = "WEBVTT - title\n\nNOTE comment\nspanning lines\n\nSTYLE\n::cue { color: red }\n\n\
intro\n00:01.000 --> 00:03.000 align:start position:10%\n<v Roger>Hello <c.yellow>there</c> &amp; <00:02.000>you\n\n\
00:01:04.000 --> 00:01:05.500\nSecond\n";
    let d = tmp();
    put(d.path(), "영상.vtt", vtt.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(
        tracks[0].cues,
        [
            cue(1000, 3000, "Hello there & you"),
            cue(64000, 65500, "Second")
        ]
    );
}

#[test]
fn t_sub_09_vtt_without_header_is_excluded() {
    // Arrange
    let body = "00:01.000 --> 00:03.000\nhi\n";
    let d = tmp();
    put(d.path(), "영상.vtt", body.as_bytes()); // WEBVTT 헤더 없음
    put(
        d.path(),
        "영상.ko.vtt",
        b"WEBVTTX\n\n00:01.000 --> 00:03.000\nhi\n",
    ); // 헤더 뒤 잡문자
    put(
        d.path(),
        "영상.en.vtt",
        format!("WEBVTT\n\n{body}").as_bytes(),
    );
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(file_names(&tracks), ["영상.en.vtt"]);
}

#[test]
fn t_sub_10_ass_keeps_comma_text_and_removes_overrides_and_drawings() {
    // Arrange
    let ass = "[Script Info]\nTitle: x\n\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: Default,Arial\n\n[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,{\\an8}Hello, world\\Nsecond{\\i1} line\n\
Comment: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,ignored\n\
Dialogue: 0,0:00:06.00,0:00:07.00,Default,,0,0,0,,{\\p1}m 0 0 l 10 10{\\p0}\n\
Dialogue: 0,0:00:08.00,0:00:09.00,Default,,0,0,0,,{\\pos(1,2)}a\\hb\n";
    let d = tmp();
    put(d.path(), "영상.ssa", ass.as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(
        tracks[0].cues,
        [
            cue(1000, 3500, "Hello, world\nsecond line"),
            cue(8000, 9000, "a b")
        ]
    );
}

#[test]
fn t_sub_10_ass_without_events_format_yields_no_track() {
    // Arrange
    let d = tmp();
    put(
        d.path(),
        "영상.ass",
        b"[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,x\n",
    );
    // Act + Assert
    assert!(load(d.path(), "영상.mp4").is_empty());
}

#[test]
fn t_sub_11_track_with_too_many_cues_is_excluded() {
    // Arrange
    let block = "1\n00:00:00,000 --> 00:00:01,000\nx\n\n";
    let d = tmp();
    put(d.path(), "영상.srt", block.repeat(MAX_CUES + 1).as_bytes());
    put(d.path(), "영상.ko.srt", block.repeat(MAX_CUES).as_bytes());
    // Act
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(file_names(&tracks), ["영상.ko.srt"]);
    assert_eq!(tracks[0].cues.len(), MAX_CUES);
}

fn smi_with(syncs: impl Iterator<Item = String>) -> String {
    let mut s = String::from("<SAMI><BODY>\n");
    s.extend(syncs);
    s.push_str("</BODY></SAMI>");
    s
}

#[test]
fn t_sub_17_sami_class_cap_excludes_file_without_scanning_everything() {
    // Arrange: 17 classes (> MAX_SAMI_CLASSES) vs 16 classes, and 100k distinct classes
    let n_classes = |n: usize| {
        smi_with((0..n).map(|i| format!("<SYNC Start={}><P Class=C{i}>t\n", i * 10 + 10)))
    };
    let d = tmp();
    put(d.path(), "영상.srt", SRT_OK.as_bytes());
    put(d.path(), "영상.ko.smi", n_classes(17).as_bytes());
    put(d.path(), "영상.en.smi", n_classes(16).as_bytes());
    put(d.path(), "영상.ja.smi", n_classes(100_000).as_bytes());
    // Act
    let started = std::time::Instant::now();
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    let names = file_names(&tracks);
    assert_eq!(names.iter().filter(|n| *n == "영상.en.smi").count(), 16);
    assert!(
        !names
            .iter()
            .any(|n| n == "영상.ko.smi" || n == "영상.ja.smi")
    );
    assert!(names.contains(&"영상.srt".to_owned()));
    assert!(
        started.elapsed().as_secs() < 30,
        "class explosion must not be quadratic"
    );
}

#[test]
fn t_sub_17_sami_many_syncs_parse_and_cue_cap() {
    // Arrange: 90k SYNC x 2 classes (정상), 250k SYNC 1 class (큐 상한 초과)
    let both = smi_with((0..90_000).map(|i| {
        format!(
            "<SYNC Start={}><P Class=KRCC>k{i}<P Class=ENCC>e{i}\n",
            i * 10 + 10
        )
    }));
    let too_many =
        smi_with((0..250_000).map(|i| format!("<SYNC Start={}><P Class=KRCC>k\n", i * 10 + 10)));
    let d = tmp();
    put(d.path(), "영상.smi", both.as_bytes());
    put(d.path(), "영상.ko.smi", too_many.as_bytes());
    // Act
    let started = std::time::Instant::now();
    let tracks = load(d.path(), "영상.mp4");
    // Assert
    assert_eq!(ids(&tracks), ["영상.smi#ENCC", "영상.smi#KRCC"]);
    assert!(tracks.iter().all(|t| t.cues.len() == 90_000));
    assert_eq!(tracks[0].cues[89_999].end_ms, 89_999 * 10 + 10 + 5000); // 마지막 큐 +5 s
    assert!(started.elapsed().as_secs() < 30);
}

#[test]
fn t_sub_18_sami_huge_start_does_not_overflow() {
    // Arrange
    let smi = format!(
        "<SAMI><BODY><SYNC Start=1000><P Class=ENCC>ok<SYNC Start={}><P Class=KRCC>max</BODY></SAMI>",
        i64::MAX
    );
    let d = tmp();
    put(d.path(), "영상.smi", smi.as_bytes());
    // Act (debug 빌드에서 덧셈 오버플로면 패닉)
    let tracks = load(d.path(), "영상.mp4");
    // Assert: 끝이 시작과 같아져(포화) 버려진 KRCC 는 큐 0개라 트랙에서 빠진다
    assert_eq!(ids(&tracks), ["영상.smi#ENCC"]);
    assert_eq!(tracks[0].cues[0].end_ms, 1000 + 5000);
}
