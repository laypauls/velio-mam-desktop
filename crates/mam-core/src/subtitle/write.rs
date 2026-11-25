//! 정규화된 큐 → SRT 텍스트 (플레이어 워커 mpv `sub-add` 임시 파일용). 이 모듈은 파일을 쓰지 않는다.

use super::Cue;
use std::fmt::Write;

/// UTF-8 BOM 으로 시작하는 SRT 문자열
pub fn to_srt(cues: &[Cue]) -> String {
    let mut out = String::from('\u{feff}');
    for (i, c) in cues.iter().enumerate() {
        let _ = write!(
            out,
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            clock(c.start_ms),
            clock(c.end_ms),
            markup_safe(&c.text)
        );
    }
    out
}

fn clock(ms: i64) -> String {
    let ms = ms.max(0);
    format!(
        "{:02}:{:02}:{:02},{:03}",
        ms / 3_600_000,
        ms / 60_000 % 60,
        ms / 1000 % 60,
        ms % 1000
    )
}

/// SRT 파서는 `<` 를 마크업(`<b>` 등) 시작으로 읽을 수 있다 - 옛 libVLC subsdec 는 그 뒤 텍스트를 삼키거나 깨뜨렸다(PoC P6 실측, libVLC 한정: "A<B" → "A", "x < y & z > w" → "x <y> w"). libmpv 에서는 재측정하지 않았으므로 보수적으로 유지한다.
/// 엔티티(`&amp;` 등)는 해석하지 않고 글자 그대로 보이므로 `&` 는 건드리지 않는다(P6, libVLC 기준). 그래서 꺾쇠만 모양이 비슷한 낫표 부호로 바꾼다.
fn markup_safe(text: &str) -> String {
    text.replace('<', "\u{2039}").replace('>', "\u{203a}")
}
