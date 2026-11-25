//! 엔진 공통 타입 — 검색 결과, 양자화, 검색 파라미터, 입력 검증.

use crate::error::{Result, VectorError};

/// 한 번에 요청할 수 있는 최대 `k` (하이브리드 후보 300~500 + 여유).
pub const MAX_K: usize = 10_000;

/// 임베딩 키 — SQLite `embedding_id` 를 그대로 쓴다 (재사용 금지).
pub type EmbeddingId = u64;

/// 인덱스 저장 정밀도. 거리 함수는 항상 코사인(입력을 정규화해 내적)이다.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantization {
    F32,
    F16,
    I8,
}

impl Quantization {
    pub fn as_str(self) -> &'static str {
        match self {
            Quantization::F32 => "f32",
            Quantization::F16 => "f16",
            Quantization::I8 => "i8",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "f32" => Some(Self::F32),
            "f16" => Some(Self::F16),
            "i8" => Some(Self::I8),
            _ => None,
        }
    }
}

/// 인덱스를 여는 방식.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenMode {
    /// 전부 RAM 에 올린다 (쓰기 가능).
    Load,
    /// 파일을 메모리 매핑한다 (읽기 전용, RSS 는 접근한 페이지만큼).
    View,
}

/// 검색 결과 한 건. `score` 는 코사인 유사도(클수록 가깝다).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub id: EmbeddingId,
    pub score: f32,
}

/// 검색 시점 파라미터. 전수 비교는 무시한다.
#[derive(Clone, Copy, Debug)]
pub struct SearchParams {
    /// HNSW 탐색 폭 (`ef_search`). `k` 보다 작으면 `k` 로 올려 쓴다.
    pub ef_search: usize,
}

impl Default for SearchParams {
    fn default() -> Self {
        Self { ef_search: 64 }
    }
}

/// 검색 중 적용하는 허용 판정 (F1). `true` 인 `embedding_id` 만 결과에 들어간다.
pub type Filter<'a> = &'a (dyn Fn(EmbeddingId) -> bool + Sync);

/// 모든 엔진이 공유하는 질의 검증: `k` 범위, 차원, 유한성, 영벡터.
pub(crate) fn validate_query(dim: usize, q: &[f32], k: usize) -> Result<()> {
    if k == 0 || k > MAX_K {
        return Err(VectorError::InvalidInput(format!(
            "k must be in 1..={MAX_K}, got {k}"
        )));
    }
    validate_vector(dim, q)
}

pub(crate) fn validate_vector(dim: usize, v: &[f32]) -> Result<()> {
    if v.len() != dim {
        return Err(VectorError::DimensionMismatch {
            expected: dim,
            found: v.len(),
        });
    }
    let mut norm2 = 0f32;
    for x in v {
        if !x.is_finite() {
            return Err(VectorError::InvalidInput(
                "vector contains NaN or Inf".into(),
            ));
        }
        norm2 += x * x;
    }
    if norm2 <= f32::MIN_POSITIVE {
        return Err(VectorError::InvalidInput(
            "zero vector is not allowed for cosine metric".into(),
        ));
    }
    Ok(())
}

/// 단위 길이로 정규화한 복사본 (검증 통과 후에만 호출).
pub fn normalized(v: &[f32]) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / norm).collect()
}

/// 엔진 공통 인터페이스. 쓰기도 `&self` — 내부에서 잠금하므로 add 중 검색이 가능하다 (T-vec-12).
pub trait AnnIndex: Send + Sync {
    fn engine_name(&self) -> &'static str;
    fn dimension(&self) -> usize;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn quantization(&self) -> Quantization;

    /// 같은 id 가 있으면 교체한다 (멱등). 입력은 정규화해 저장한다.
    fn add(&self, id: EmbeddingId, v: &[f32]) -> Result<()>;
    /// 없는 id 도 오류가 아니다. 제거했으면 `true`.
    fn remove(&self, id: EmbeddingId) -> Result<bool>;
    /// 코사인 유사도 내림차순 상위 `k`. `filter` 가 있으면 검색 중 적용(F1).
    fn search(
        &self,
        q: &[f32],
        k: usize,
        params: &SearchParams,
        filter: Option<Filter<'_>>,
    ) -> Result<Vec<Hit>>;
    /// 엔진이 보고하는 사용 메모리(바이트, 근사). view 로 열었으면 매핑 크기를 뺀 값은 보장하지 않는다.
    fn memory_bytes(&self) -> usize;
    /// 엔진 데이터 파일을 `path` 에 쓴다 (메타 파일은 [`crate::store`] 가 쓴다).
    fn save_file(&self, path: &std::path::Path) -> Result<()>;
}

/// 여러 스레드로 `rows`(행 우선, `ids.len() × dim`)를 한꺼번에 넣는다. 구축·재구축 측정에 쓴다.
/// 엔진의 `add` 가 내부에서 잠금하므로 그대로 병렬 호출한다 (flat 은 쓰기 잠금 때문에 사실상 직렬).
pub fn add_parallel(
    index: &dyn AnnIndex,
    ids: &[EmbeddingId],
    rows: &[f32],
    threads: usize,
) -> Result<()> {
    let dim = index.dimension();
    if rows.len() != ids.len() * dim {
        return Err(VectorError::InvalidInput(
            "rows length must be ids.len() * dimension".into(),
        ));
    }
    let t = threads.max(1).min(ids.len().max(1));
    if t == 1 {
        for (i, &id) in ids.iter().enumerate() {
            index.add(id, &rows[i * dim..(i + 1) * dim])?;
        }
        return Ok(());
    }
    let chunk = ids.len().div_ceil(t);
    std::thread::scope(|s| {
        let handles: Vec<_> = ids
            .chunks(chunk)
            .zip(rows.chunks(chunk * dim))
            .map(|(cid, crow)| {
                s.spawn(move || -> Result<()> {
                    for (i, &id) in cid.iter().enumerate() {
                        index.add(id, &crow[i * dim..(i + 1) * dim])?;
                    }
                    Ok(())
                })
            })
            .collect();
        handles
            .into_iter()
            .try_for_each(|h| h.join().expect("add thread panicked"))
    })
}
