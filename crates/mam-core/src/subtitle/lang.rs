//! 언어 코드 정규화 표와 SAMI 클래스 표 (닫힌 목록 — 표에 없으면 언어 코드가 아니다).

const TABLE: &[(&str, &[&str])] = &[
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

const SAMI_CLASSES: &[(&str, &str)] = &[
    ("KRCC", "ko"),
    ("KOCC", "ko"),
    ("KOKRCC", "ko"),
    ("ENCC", "en"),
    ("EGCC", "en"),
    ("ENUSCC", "en"),
    ("JPCC", "ja"),
    ("JACC", "ja"),
    ("CNCC", "zh"),
    ("ZHCC", "zh"),
    ("CHCC", "zh"),
];

/// 언어 코드·이름(대소문자 무시, `_` = `-`)을 ISO 639-1 코드로 바꾼다. 표에 없으면 `None`.
pub fn normalize_language(code: &str) -> Option<&'static str> {
    let k = code.trim().to_ascii_lowercase().replace('_', "-");
    TABLE
        .iter()
        .find(|(_, aliases)| aliases.contains(&k.as_str()))
        .map(|(canon, _)| *canon)
}

/// SAMI 클래스 이름(대소문자 무시)의 언어. 표에 없으면 `None`.
pub(super) fn sami_class_language(class: &str) -> Option<&'static str> {
    let up = class.to_ascii_uppercase();
    SAMI_CLASSES.iter().find(|(c, _)| *c == up).map(|(_, l)| *l)
}
