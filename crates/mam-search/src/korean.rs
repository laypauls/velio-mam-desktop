//! Lindera(ko-dic) 형태소 분석 + 조사·어미 제거.
//!
//! 사전 로드는 비싸므로(임베디드 ko-dic 파싱) 프로세스당 한 번만 한다.

use std::sync::OnceLock;

use lindera::dictionary::{DictionaryKind, load_embedded_dictionary};
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;

use crate::error::{Result, SearchError};
use crate::preprocess::normalize;

static SEGMENTER: OnceLock<std::result::Result<Segmenter, String>> = OnceLock::new();

fn segmenter() -> Result<&'static Segmenter> {
    SEGMENTER
        .get_or_init(|| {
            load_embedded_dictionary(DictionaryKind::KoDic)
                .map(|dict| Segmenter::new(Mode::Normal, dict, None))
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| SearchError::Dictionary(e.clone()))
}

/// 형태소 하나의 분석 결과 (소문자 표면형, 정규화된 입력 기준 바이트 오프셋, 품사).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Morph {
    pub text: String,
    pub from: usize,
    pub to: usize,
    pub pos: String,
}

/// 색인·질의에서 버리는 품사인가. ko-dic(세종 품사)의 복합 태그는 `+` 로 이어지며
/// (`XSV+EP+EF`), **모든 구성이 버릴 품사일 때만** 버린다 (`VV+EC` 는 어간이 있어 유지).
///
/// 버리는 품사: `J*` 조사, `E*` 어미, `XS*` 접미사, `VCP`/`VCN` 지정사(이다/아니다),
/// `S*` 중 기호류(`SF SP SS SE SO SC SY`, 단 `SL` 외국어·`SN` 숫자·`SH` 한자는 유지).
/// `VX`(보조용언)·`NNB`(의존명사)는 유지한다 — 의미 손실 위험이 더 크다고 보았다.
fn is_droppable(pos: &str) -> bool {
    pos.split('+').all(|p| {
        p.starts_with('J')
            || p.starts_with('E')
            || p.starts_with("XS")
            || matches!(
                p,
                "VCP" | "VCN" | "SF" | "SP" | "SS" | "SE" | "SO" | "SC" | "SY"
            )
    })
}

/// 활용형 복합 토큰(`달린다` = `달리/VV/*+ᆫ다/EC/*`)을 첫 형태소(어간)로 환원한다.
/// ko-dic 의 `type` 이 `Inflect` 이고 `expression` 의 첫 구성이 내용어일 때만 적용한다
/// (`Compound` 등 다른 유형은 건드리지 않는다). 실패하면 `None`.
fn inflect_stem(type_field: Option<&str>, expression: Option<&str>) -> Option<String> {
    if type_field? != "Inflect" {
        return None;
    }
    let first = expression?.split('+').next()?;
    let mut parts = first.split('/');
    let (morph, pos) = (parts.next()?, parts.next()?);
    (!morph.is_empty() && !is_droppable(pos)).then(|| morph.to_lowercase())
}

/// `text` 를 형태소로 나눈다 (색인 규칙). `drop_functional` 이면 조사·어미 등을 제거하고
/// 활용형은 어간으로 환원하되, 전부 제거되어 아무것도 남지 않으면 제거 전 결과를 돌려준다.
/// 질의에는 [`analyze_query`] 를 쓴다 — 기능어만 있는 단어를 질의에서 버리기 위해서다.
pub fn analyze(text: &str, drop_functional: bool) -> Result<Vec<Morph>> {
    let (all, kept) = analyze_both(text, drop_functional)?;
    Ok(if kept.is_empty() { all } else { kept })
}

/// 질의용: `analyze(text, true)` 와 같지만 **폴백이 없다** — 기능어·기호뿐인 단어(`에서`, `&`)는
/// 빈 목록을 돌려주고 호출자가 그 단어를 질의에서 버린다. 색인은 폴백을 유지하므로 규칙이 갈린다.
///
/// 또 하나의 질의 전용 규칙: ko-dic 은 일부 단음절 조사를 **명사로도** 등록해 문맥에 따라 명사로
/// 분석한다(`스키의` → `스키/NNG 의/NNG`, 같은 `의` 가 `스키의 계절` 에서는 `JKG`). 그러면 질의에
/// `의` 가 필수 토큰으로 남아 `스키` 문서를 못 찾는다. 그래서 **단어의 맨 끝** 단음절 토큰이
/// 아래 조사 표면형 목록에 있고 앞에 다른 토큰이 있으면 질의에서만 버린다 (재현율 우선 —
/// 색인 쪽 토큰은 그대로라 색인 버전은 올리지 않는다).
pub fn analyze_query(text: &str) -> Result<Vec<Morph>> {
    let (all, mut kept) = analyze_both(text, true)?;
    if kept.len() >= 2
        && let (Some(last), Some(kept_last)) = (all.last(), kept.last())
        && last.to == kept_last.to
        && TRAILING_PARTICLES.contains(&kept_last.text.as_str())
    {
        kept.pop();
    }
    Ok(kept)
}

/// 단어 끝에서 명사로 분석될 수 있는 단음절 조사 표면형 (질의 전용 보정).
const TRAILING_PARTICLES: [&str; 14] = [
    "의", "이", "가", "은", "는", "을", "를", "에", "도", "로", "과", "와", "만", "께",
];

fn analyze_both(text: &str, drop_functional: bool) -> Result<(Vec<Morph>, Vec<Morph>)> {
    let normalized = normalize(text);
    if normalized.trim().is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let seg = segmenter()?;
    let tokens = seg
        .segment(std::borrow::Cow::Borrowed(normalized.as_str()))
        .map_err(|e| SearchError::Dictionary(e.to_string()))?;
    let mut all = Vec::with_capacity(tokens.len());
    for mut t in tokens {
        let pos = t.get_detail(0).unwrap_or("*").to_string();
        let surface = t.surface.trim().to_lowercase();
        if surface.is_empty() {
            continue;
        }
        let text = if drop_functional {
            let ty = t.get_detail(4).map(str::to_string);
            let expr = t.get_detail(7).map(str::to_string);
            inflect_stem(ty.as_deref(), expr.as_deref()).unwrap_or(surface)
        } else {
            surface
        };
        all.push(Morph {
            text,
            from: t.byte_start,
            to: t.byte_end,
            pos,
        });
    }
    if !drop_functional {
        return Ok((all, Vec::new()));
    }
    let kept = all
        .iter()
        .filter(|m| !is_droppable(&m.pos))
        .cloned()
        .collect();
    Ok((all, kept))
}

/// 사전을 미리 로드해 실패를 조기에 오류로 받는다 (`token_stream` 은 오류를 삼키므로).
pub fn ensure_ready() -> Result<()> {
    segmenter().map(|_| ())
}
