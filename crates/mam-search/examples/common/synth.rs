//! 합성 자산 생성 규칙 (파일명 패턴·태그·AI 텍스트·메타데이터). 모두 `Rng` 시드로 결정된다.

use std::collections::HashSet;

use super::rng::{Rng, civil_ms};
use super::vocab::{FN_EN, FN_KO, FN_PLACES, Josa, TEXT_EN, TEXT_KO, TEXT_PROPER, with_josa};

/// 측정용 고정 "현재" (시계를 읽지 않는다): 2026-10-25 00:00 UTC.
pub const NOW_MS: i64 = 1_792_886_400_000;

pub struct VolumeSpec {
    pub id: i64,
    pub kind: &'static str,
    pub label: &'static str,
    pub mount: &'static str,
    pub online: bool,
    pub weight: u32,
}

macro_rules! vol {
    ($id:expr, $kind:expr, $label:expr, $mount:expr, $online:expr, $w:expr) => {
        VolumeSpec {
            id: $id,
            kind: $kind,
            label: $label,
            mount: $mount,
            online: $online,
            weight: $w,
        }
    };
}

/// 14개 볼륨 — 고정 4 / 이동식 6 / 네트워크 4, 그중 3개가 오프라인(이동식 2, 네트워크 1).
pub const VOLUMES: [VolumeSpec; 14] = [
    vol!(1, "fixed", "SystemSSD", "C:\\", true, 35),
    vol!(2, "fixed", "DataHDD", "D:\\", true, 25),
    vol!(3, "fixed", "Work", "F:\\", true, 6),
    vol!(4, "fixed", "Scratch", "G:\\", true, 4),
    vol!(5, "removable", "USB-Red", "E:\\", true, 6),
    vol!(6, "removable", "USB-Blue", "H:\\", true, 5),
    vol!(7, "removable", "SD-Camera1", "I:\\", false, 4),
    vol!(8, "removable", "SD-Camera2", "J:\\", true, 3),
    vol!(9, "removable", "Backup-2019", "K:\\", false, 3),
    vol!(10, "removable", "Drone-SD", "L:\\", true, 2),
    vol!(11, "network", "NAS-Photos", "\\\\nas\\photos", true, 5),
    vol!(12, "network", "NAS-Video", "\\\\nas\\video", true, 3),
    vol!(13, "network", "NAS-Archive", "\\\\nas2\\archive", false, 3),
    vol!(14, "network", "Office-Share", "\\\\office\\share", true, 1),
];

pub fn volume_weights() -> Vec<u32> {
    VOLUMES.iter().map(|v| v.weight).collect()
}

/// 태그 어휘 (인기 순: 한국어 -> 고유명사 -> 영어). `tags` 테이블에 모두 넣는다.
pub fn tag_list() -> Vec<&'static str> {
    TEXT_KO
        .iter()
        .chain(TEXT_PROPER)
        .chain(TEXT_EN)
        .copied()
        .collect()
}

pub struct NewAsset {
    pub media_type: &'static str,
    pub root: &'static str,
    pub file_name: String,
    pub file_size: i64,
    pub modified_at: i64,
    pub captured_at: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub codec: &'static str,
    pub hashed: bool,
    pub missing: bool,
}

const YEAR_WEIGHTS: [u32; 12] = [1, 2, 3, 4, 5, 7, 9, 11, 13, 17, 20, 9]; // 2015..=2026

fn random_capture(rng: &mut Rng) -> (i64, [i64; 6]) {
    let y = 2015 + rng.weighted(&YEAR_WEIGHTS) as i64;
    let m = if y == 2026 {
        rng.range(1, 10)
    } else {
        rng.range(1, 12)
    } as i64;
    let d = rng.range(1, 28) as i64;
    let (hh, mm, ss) = (
        rng.range(0, 23) as i64,
        rng.range(0, 59) as i64,
        rng.range(0, 59) as i64,
    );
    (civil_ms(y, m, d, hh, mm, ss), [y, m, d, hh, mm, ss])
}

fn pick_ext(rng: &mut Rng, media: &str) -> String {
    let (exts, w): (&[&str], &[u32]) = match media {
        "video" => (&["mp4", "mov", "mkv", "avi"], &[60, 25, 10, 5]),
        "audio" => (&["mp3", "m4a", "wav", "flac", "ogg"], &[35, 30, 20, 10, 5]),
        _ => (
            &["jpg", "jpeg", "png", "heic", "webp", "cr2"],
            &[55, 5, 15, 10, 5, 3],
        ),
    };
    let e = exts[rng.weighted(w)];
    if rng.chance(0.15) {
        e.to_uppercase()
    } else {
        e.to_string()
    }
}

fn nn(rng: &mut Rng) -> String {
    format!("{:02}", rng.range(1, 99))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}

fn korean_name(rng: &mut Rng, ymd: &[i64; 6]) -> String {
    let place = *rng.pick(FN_PLACES);
    let a = *rng.pick(FN_KO);
    let b = *rng.pick(FN_KO);
    match rng.below(6) {
        0 => format!("{place}{a}_{b}_{}", nn(rng)),
        1 => format!("{a}_{}", nn(rng)),
        2 => format!("{place}_{a}_{:04}{:02}{:02}", ymd[0], ymd[1], ymd[2]),
        3 => format!("{a} ({})", rng.range(1, 9)),
        4 => format!("{:04}-{:02}-{:02} {place}{a}", ymd[0], ymd[1], ymd[2]),
        _ => format!("{place}{a} {b} {}", nn(rng)),
    }
}

fn english_name(rng: &mut Rng) -> String {
    let a = *rng.pick(FN_EN);
    let b = *rng.pick(FN_EN);
    match rng.below(4) {
        0 => format!("{a}_{b}_{}", nn(rng)),
        1 => format!("{}{}_{}", capitalize(a), capitalize(b), nn(rng)),
        2 => format!("{a}-{b}-v{}", rng.range(1, 5)),
        _ => format!("{a} {b} ({})", rng.range(1, 9)),
    }
}

fn mixed_name(rng: &mut Rng, ymd: &[i64; 6]) -> String {
    let place = *rng.pick(FN_PLACES);
    let ko = *rng.pick(FN_KO);
    let en = *rng.pick(FN_EN);
    match rng.below(3) {
        0 => format!("{place}{en}_{}", nn(rng)),
        1 => format!("{ko}_{en}_{}", nn(rng)),
        _ => format!("{ko}{en}_{:02}{:02}", ymd[1], ymd[2]),
    }
}

fn short_name(rng: &mut Rng) -> String {
    const SHORT: &[&str] = &[
        "a", "b", "x1", "01", "7", "ab", "가", "나", "가나", "다1", "z", "99", "m2", "k", "d3",
    ];
    rng.pick(SHORT).to_string()
}

fn date_name(ymd: &[i64; 6]) -> String {
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}",
        ymd[0], ymd[1], ymd[2], ymd[3], ymd[4], ymd[5]
    )
}

/// 한 자산의 파일명(확장자 포함)과 메타데이터. `captured` 는 날짜형 이름이 쓰면 반드시 채워진다.
pub fn gen_asset(rng: &mut Rng) -> NewAsset {
    let media_type = ["image", "video", "audio"][rng.weighted(&[60, 30, 10])];
    let (cap_ms, ymd) = random_capture(rng);
    let ext = pick_ext(rng, media_type);
    let mut has_capture = rng.chance(0.88) && media_type != "audio";
    let pattern = if media_type == "audio" {
        if rng.chance(0.7) {
            10
        } else {
            [5, 6][rng.below(2)]
        }
    } else {
        rng.weighted(&[22, 8, 14, 6, 6, 24, 8, 6, 3])
    };
    let stem = match pattern {
        0 => match media_type {
            "video" => format!(
                "{}_{:04}",
                if rng.chance(0.5) { "MVI" } else { "VID" },
                rng.range(1, 9999)
            ),
            _ => format!("IMG_{:04}", rng.range(1, 9999)),
        },
        1 if media_type == "image" => format!("DSC{:05}", rng.range(1, 99999)),
        1 => format!("MVI_{:04}", rng.range(1, 9999)),
        2 => {
            has_capture = true;
            date_name(&ymd)
        }
        3 => {
            has_capture = true;
            let p = if media_type == "video" { "VID" } else { "IMG" };
            format!("{p}_{}", date_name(&ymd))
        }
        4 if media_type == "image" => {
            has_capture = true;
            if rng.chance(0.5) {
                format!(
                    "Screenshot_{:04}-{:02}-{:02}_{:02}{:02}{:02}",
                    ymd[0], ymd[1], ymd[2], ymd[3], ymd[4], ymd[5]
                )
            } else {
                format!(
                    "스크린샷 {:04}-{:02}-{:02} {:02}{:02}{:02}",
                    ymd[0], ymd[1], ymd[2], ymd[3], ymd[4], ymd[5]
                )
            }
        }
        4 => {
            has_capture = true;
            date_name(&ymd)
        }
        5 => korean_name(rng, &ymd),
        6 => english_name(rng),
        7 => mixed_name(rng, &ymd),
        8 => short_name(rng),
        _ => match rng.below(4) {
            0 => format!("녹음 {}", rng.range(1, 300)),
            1 => format!("Recording_{}", rng.range(1, 300)),
            2 => format!("voice_note_{:04}", rng.range(1, 9999)),
            _ => format!("Recording {}", date_name(&ymd)),
        },
    };
    let (root, width, height, duration_ms, codec, file_size) = match media_type {
        "image" => {
            let (w, h) = *rng.pick(&[
                (4032, 3024),
                (3024, 4032),
                (1920, 1080),
                (1080, 1920),
                (6000, 4000),
                (2560, 1440),
                (800, 600),
            ]);
            let size = if rng.chance(0.05) {
                rng.range(8_000_000, 40_000_000)
            } else {
                rng.range(200_000, 8_000_000)
            };
            (
                *rng.pick(&["사진", "Photos", "Camera"]),
                Some(w),
                Some(h),
                None,
                *rng.pick(&["jpeg", "heic", "png", "webp", "raw"]),
                size as i64,
            )
        }
        "video" => {
            let (w, h) = *rng.pick(&[(1920, 1080), (3840, 2160), (1280, 720), (1080, 1920)]);
            let dur = if rng.chance(0.8) {
                rng.range(3_000, 30_000)
            } else {
                rng.range(30_000, 7_200_000)
            };
            let size = if rng.chance(0.05) {
                rng.range(500_000_000, 4_000_000_000)
            } else {
                rng.range(5_000_000, 500_000_000)
            };
            (
                *rng.pick(&["Videos", "영상"]),
                Some(w),
                Some(h),
                Some(dur as i64),
                *rng.pick(&["h264", "hevc", "vp9", "mpeg4"]),
                size as i64,
            )
        }
        _ => (
            *rng.pick(&["Audio", "녹음"]),
            None,
            None,
            Some(rng.range(10_000, 3_600_000) as i64),
            *rng.pick(&["aac", "mp3", "pcm", "flac", "opus"]),
            rng.range(500_000, 50_000_000) as i64,
        ),
    };
    NewAsset {
        media_type,
        root,
        file_name: format!("{stem}.{ext}"),
        file_size,
        modified_at: cap_ms + rng.range(0, 86_400_000) as i64,
        captured_at: has_capture.then_some(cap_ms),
        width,
        height,
        duration_ms,
        codec,
        hashed: rng.chance(0.9),
        missing: rng.chance(0.02),
    }
}

/// 같은 폴더(배치)에서 이름이 겹치지 않게 한다 — `(volume_id, path_key)` 유일 제약 보호.
pub fn dedupe_name(used: &mut HashSet<String>, root: &str, name: String) -> String {
    let mut candidate = name.clone();
    let mut n = 2;
    while !used.insert(format!("{root}/{}", candidate.to_lowercase())) {
        match name.rfind('.') {
            Some(dot) => candidate = format!("{}_{n}{}", &name[..dot], &name[dot..]),
            None => candidate = format!("{name}_{n}"),
        }
        n += 1;
    }
    candidate
}

const TAG_COUNT_WEIGHTS: [u32; 9] = [15, 15, 15, 15, 12, 10, 8, 5, 5]; // 0..=8

/// 자산 하나의 태그 인덱스(`tag_list` 기준, 중복 없음).
pub fn gen_tags(rng: &mut Rng, tag_total: usize) -> Vec<usize> {
    let want = rng.weighted(&TAG_COUNT_WEIGHTS);
    let mut out: Vec<usize> = Vec::with_capacity(want);
    let mut guard = 0;
    while out.len() < want && guard < 50 {
        let t = rng.skewed(tag_total);
        if !out.contains(&t) {
            out.push(t);
        }
        guard += 1;
    }
    out
}

pub struct AiText {
    pub description: Option<String>,
    pub keywords: Option<String>,
    pub memo: Option<String>,
}

fn noun(rng: &mut Rng) -> &'static str {
    TEXT_KO[rng.skewed(TEXT_KO.len())]
}

fn sentence(rng: &mut Rng) -> String {
    let (a, b, c) = (noun(rng), noun(rng), noun(rng));
    match rng.below(5) {
        0 => format!(
            "{} {b}에서 {} 바라보고 있다.",
            with_josa(a, Josa::Subject),
            with_josa(c, Josa::Object)
        ),
        1 => format!(
            "{} {} 함께 {c}에서 시간을 보내는 모습.",
            with_josa(a, Josa::With),
            with_josa(b, Josa::Subject)
        ),
        2 => format!(
            "{} 배경으로 {} 서 있다.",
            with_josa(c, Josa::Object),
            with_josa(a, Josa::Subject)
        ),
        3 => format!(
            "{a}의 {} {} 잘 어울린다.",
            with_josa(b, Josa::Topic),
            with_josa(c, Josa::With)
        ),
        _ => format!(
            "{} {} 찾아 {} 걸어간다.",
            with_josa(a, Josa::Subject),
            with_josa(b, Josa::Object),
            with_josa(c, Josa::Toward)
        ),
    }
}

/// AI 설명·키워드(보유 비율 `ai_fraction`)와 사용자 메모(3%). 둘 다 없으면 `None`.
pub fn gen_text(rng: &mut Rng, ai_fraction: f64) -> Option<AiText> {
    let ai = rng.chance(ai_fraction);
    let memo = rng.chance(0.03);
    if !ai && !memo {
        return None;
    }
    let (description, keywords) = if ai {
        let sentences: Vec<String> = (0..rng.range(1, 3)).map(|_| sentence(rng)).collect();
        let tags = tag_list();
        let mut kws: Vec<&str> = Vec::new();
        for _ in 0..rng.range(3, 6) {
            let k = tags[rng.skewed(tags.len())];
            if !kws.contains(&k) {
                kws.push(k);
            }
        }
        (Some(sentences.join(" ")), Some(kws.join(", ")))
    } else {
        (None, None)
    };
    let memo = memo.then(|| {
        format!(
            "메모: {} 다시 확인할 것",
            with_josa(noun(rng), Josa::Object)
        )
    });
    Some(AiText {
        description,
        keywords,
        memo,
    })
}
