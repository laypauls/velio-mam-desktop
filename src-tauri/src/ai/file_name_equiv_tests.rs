//! T-pgw-11b: 플러그인 호스트 `asset.get` 의 파일 이름 정제(`mam_core::assets::sanitize_file_name`)가 외부 AI 전송 계층의
//! `SanitizedMeta::new` 와 같은 결과를 내는지 확인한다 (두 crate 를 모두 아는 곳이 앱뿐이라 여기에 둔다).
//! 분류: Validation / Boundary(빈 이름은 `SanitizedMeta` 만 "unknown" 으로 대체 — 플러그인에는 null)

use mam_ai::external::{MediaType, SanitizedMeta};
use mam_core::assets::sanitize_file_name;

#[test]
fn t_pgw_11b_file_name_sanitizer_matches_sanitized_meta() {
    let long = format!("dir/{}", "가".repeat(300));
    let cases = [
        "photo.jpg",
        "a/b/c.jpg",
        "a\\b\\c.jpg",
        "x/na\u{7}me\u{1b}.jpg",
        "d/line\nbreak\r.png",
        "mixed\\and/slash\\final name.mp4",
        "tab\tinside.wav",
        "\u{202e}rtl.jpg",
        long.as_str(),
    ];
    for raw in cases {
        let want = SanitizedMeta::new(raw, None, MediaType::Image, None).file_name;
        assert_eq!(sanitize_file_name(raw).as_deref(), Some(want.as_str()), "{raw:?}");
    }
    // 빈 이름: SanitizedMeta 는 "unknown", 플러그인 쪽은 None
    for raw in ["", "dir/", "d/\u{1}\u{2}"] {
        assert_eq!(SanitizedMeta::new(raw, None, MediaType::Image, None).file_name, "unknown");
        assert_eq!(sanitize_file_name(raw), None, "{raw:?}");
    }
}
