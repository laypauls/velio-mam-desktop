//! 인덱스 디렉터리의 `mam-search.meta` — 인덱스를 만든 형식·분석기 버전·전략 옵션.
//!
//! Tantivy 는 분석기 구성과 필드 채움 여부를 영속하지 않는다. 다른 옵션으로 다시 열면
//! 질의가 조용히 0건을 돌려주므로(예: A 로 만든 인덱스를 `all()` 로 열기), 열 때 대조해
//! 다르면 [`SearchError::IndexMismatch`] 를 반환한다. 분석기·스키마를 바꾸면
//! `ANALYZER_VERSION` 을 올려 기존 인덱스를 재구축 대상으로 만든다.

use std::path::Path;

use crate::error::{Result, SearchError};
use crate::index::IndexOptions;
use crate::retry_dir::{RetryPolicy, retry_io};

/// 메타 파일·디렉터리 배치 형식 버전.
pub const FORMAT_VERSION: u32 = 1;
/// 분석기(토큰화 규칙)·스키마 버전. 색인 결과가 달라지는 변경 시 올린다.
/// v2: 활용형 어간 환원 추가(I7).
/// v3: 사용자 데이터 필드 추가 — 즐겨찾기·컬렉션 id·태그 id (P2-7).
/// v4: 검색 조정기 필드 추가 — 수정일·이름 정렬 키·폴더 범위 토큰 (P2-5).
/// v5: AI 설명·키워드·OCR·요약·STT 전사 텍스트 채움 (P3 로더, V6 테이블).
/// v6: 사용자 제목 `title` 텍스트 + 제목순 정렬 키 `title_hi/lo` (콘텐츠 제목).
/// v7: 촬영일 대체 정렬·필터 키 `captured_or_modified` (초; 촬영일 없으면 수정일 ms/1000).
/// v8: 별점 `rating` (0..=5, 0 = 미평가) — 범위 필터·정렬.
/// v9: 사용자 정의 메타데이터 필드 `umeta`(JSON)·`umeta_has`·`umeta_text` 추가.
pub const ANALYZER_VERSION: u32 = 9;

const FILE: &str = "mam-search.meta";

fn render(o: &IndexOptions) -> String {
    format!(
        "format_version={FORMAT_VERSION}\nanalyzer_version={ANALYZER_VERSION}\nbuild_ngram={}\nbuild_raw={}\n",
        o.build_ngram, o.build_raw
    )
}

/// 메타가 있으면 대조하고, 새 디렉터리면 먼저 기록한다 (기록 후 중단돼도 다음 open 이 이어서 만든다).
pub(crate) fn check_or_write(dir: &Path, options: &IndexOptions) -> Result<()> {
    let expected = render(options);
    let path = dir.join(FILE);
    match std::fs::read_to_string(&path) {
        Ok(found) => {
            let found = found.replace("\r\n", "\n");
            if found != expected {
                return Err(SearchError::IndexMismatch(format!(
                    "found [{}] expected [{}]",
                    found.trim().replace('\n', ", "),
                    expected.trim().replace('\n', ", ")
                )));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if dir.join("meta.json").exists() {
                return Err(SearchError::IndexMismatch(format!(
                    "index exists without {FILE}"
                )));
            }
            let tmp = dir.join(format!("{FILE}.tmp"));
            // 방금 만든 파일을 백신이 잡고 있으면 rename 이 접근 거부로 간헐 실패한다 → 재시도
            let policy = RetryPolicy::default();
            retry_io(policy, || std::fs::write(&tmp, &expected))?;
            retry_io(policy, || std::fs::rename(&tmp, &path))?;
            Ok(())
        }
        // 비 UTF-8 로 손상된 메타 -> 다른 손상처럼 재구축 대상 (영구 열기 실패 금지)
        Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Err(SearchError::IndexMismatch(
            format!("{FILE} is not valid UTF-8: {e}"),
        )),
        Err(e) => Err(e.into()),
    }
}
