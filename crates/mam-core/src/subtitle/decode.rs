//! 자막 바이트 → UTF-8 문자열. BOM → UTF-8 유효성 → CP949(EUC-KR) 순.

use encoding_rs::{EUC_KR, UTF_16BE, UTF_16LE};

/// 해석할 수 없으면 `None` (호출자가 그 파일만 제외한다).
pub(super) fn decode(bytes: &[u8]) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        std::str::from_utf8(rest).ok()?.to_owned()
    } else if bytes.starts_with(&[0xFF, 0xFE]) {
        strict(UTF_16LE.decode_with_bom_removal(bytes))?
    } else if bytes.starts_with(&[0xFE, 0xFF]) {
        strict(UTF_16BE.decode_with_bom_removal(bytes))?
    } else if let Ok(s) = std::str::from_utf8(bytes) {
        s.to_owned()
    } else {
        strict(EUC_KR.decode_without_bom_handling(bytes))?
    };
    // NUL 이 섞인 바이너리(예: VobSub)는 자막이 아니다
    (!text.contains('\0')).then_some(text)
}

fn strict((s, had_errors): (std::borrow::Cow<'_, str>, bool)) -> Option<String> {
    (!had_errors).then(|| s.into_owned())
}
