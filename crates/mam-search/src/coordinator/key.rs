//! 정렬 순위 키와 커서 인코딩.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tantivy::collector::sort_key::NaturalComparator;
use tantivy::collector::{SegmentSortKeyComputer, SortKeyComputer};
use tantivy::columnar::Column;
use tantivy::{DocId, Score, SegmentReader};

use super::collection_order::CollectionOrder;
use super::{CoordRequest, SortField, SortSpec};
use crate::error::{Result, SearchError};

/// `[present, k1, k2, tie]` — 사전식 비교에서 클수록 앞. `present = 0` 은 정렬 값이 없는 문서(항상 뒤).
pub(crate) type RankKey = [u64; 4];

/// 커서 이후 조건에 걸러진 문서의 표지. 정상 키는 `tie >= 1` 이라 겹치지 않는다.
pub(crate) const EXCLUDED: RankKey = [0, 0, 0, 0];

/// `asset_id` 오름차순이 되도록 뒤집는다.
fn tie_of(asset_id: u64) -> u64 {
    u64::MAX - asset_id
}

pub(crate) fn tie_to_asset(k: &RankKey) -> u64 {
    u64::MAX - k[3]
}

pub(super) fn ord_i64(v: i64) -> u64 {
    (v as u64) ^ (1 << 63)
}

enum Primary {
    Score,
    None,
    I64(Option<Column<i64>>),
    U64(Option<Column<u64>>),
    Name(Option<Column<u64>>, Option<Column<u64>>),
    /// 컬렉션 순서 맵 (맵에 없는 문서는 `present = 0`)
    Position(Arc<CollectionOrder>),
}

pub(crate) struct KeyComputer {
    sort: SortSpec,
    relevance: bool,
    after: Option<RankKey>,
    /// true 면 순서를 뒤집는다 (모든 성분의 비트 반전). 커서 규칙도 반대: 키가 `after` **이하**인 문서를 뺀다.
    /// 반환 키는 반전된 값이다 — 호출자가 [`invert_key`] 로 되돌린다.
    invert: bool,
    /// 커서 때문에 빠진 문서 수 (순방향: 키 >= 커서, 역방향: 키 <= 커서). 세그먼트가 끝날 때 합산된다.
    skipped: Arc<AtomicU64>,
    /// `SortField::CollectionOrder` 일 때만 쓰는 순서 맵
    order: Option<Arc<CollectionOrder>>,
}

impl KeyComputer {
    pub(crate) fn new(
        sort: SortSpec,
        relevance: bool,
        after: Option<RankKey>,
        order: Option<Arc<CollectionOrder>>,
    ) -> Self {
        Self {
            sort,
            relevance,
            after,
            order,
            invert: false,
            skipped: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn inverted(mut self, invert: bool) -> Self {
        self.invert = invert;
        self
    }

    /// 커서로 제외된 문서 수를 읽는 공유 카운터.
    pub(crate) fn skipped_counter(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.skipped)
    }
}

/// [`KeyComputer::inverted`] 가 반환한 키를 원래 순서의 키로 되돌린다 (자기 역함수).
pub(crate) fn invert_key(k: &RankKey) -> RankKey {
    [!k[0], !k[1], !k[2], !k[3]]
}

pub(crate) struct KeyChild {
    asset_id: Column<u64>,
    primary: Primary,
    descending: bool,
    after: Option<RankKey>,
    invert: bool,
    skipped: Arc<AtomicU64>,
    skipped_local: u64,
}

impl Drop for KeyChild {
    fn drop(&mut self) {
        self.skipped
            .fetch_add(self.skipped_local, Ordering::Relaxed);
    }
}

impl KeyChild {
    fn dir(&self, v: u64) -> u64 {
        if self.descending { v } else { !v }
    }
}

impl SegmentSortKeyComputer for KeyChild {
    type SortKey = RankKey;
    type SegmentSortKey = RankKey;
    type SegmentComparator = NaturalComparator;

    fn segment_sort_key(&mut self, doc: DocId, score: Score) -> RankKey {
        let Some(asset_id) = self.asset_id.first(doc) else {
            return EXCLUDED;
        };
        let (present, k1, k2) = match &self.primary {
            // 점수는 0 이상이라 비트 패턴이 순서를 보존한다
            Primary::Score => (1, self.dir(u64::from(score.max(0.0).to_bits())), 0),
            Primary::None => (1, 0, 0),
            Primary::I64(col) => match col.as_ref().and_then(|c| c.first(doc)) {
                Some(v) => (1, self.dir(ord_i64(v)), 0),
                None => (0, 0, 0),
            },
            Primary::U64(col) => match col.as_ref().and_then(|c| c.first(doc)) {
                Some(v) => (1, self.dir(v), 0),
                None => (0, 0, 0),
            },
            Primary::Position(map) => match map.get(asset_id) {
                // 등급은 방향과 무관 (위치 있는 행이 먼저, NULL 행이 그 뒤). 값만 방향을 따른다
                Some((class, v)) => (class, self.dir(v), 0),
                None => (0, 0, 0),
            },
            Primary::Name(hi, lo) => {
                let h = hi.as_ref().and_then(|c| c.first(doc)).unwrap_or(0);
                let l = lo.as_ref().and_then(|c| c.first(doc)).unwrap_or(0);
                (1, self.dir(h), self.dir(l))
            }
        };
        let key = [present, k1, k2, tie_of(asset_id)];
        if let Some(after) = &self.after {
            let out = if self.invert {
                key <= *after
            } else {
                key >= *after
            };
            if out {
                self.skipped_local += 1;
                return EXCLUDED;
            }
        }
        if self.invert { invert_key(&key) } else { key }
    }

    fn convert_segment_sort_key(&self, sort_key: RankKey) -> RankKey {
        sort_key
    }
}

impl SortKeyComputer for KeyComputer {
    type SortKey = RankKey;
    type Child = KeyChild;
    type Comparator = NaturalComparator;

    fn requires_scoring(&self) -> bool {
        self.relevance
    }

    fn segment_sort_key_computer(&self, reader: &SegmentReader) -> tantivy::Result<KeyChild> {
        let ff = reader.fast_fields();
        let primary = match self.sort.field {
            SortField::Relevance if self.relevance => Primary::Score,
            // 질의 없는 관련도 정렬 = 점수가 모두 같다 -> asset_id 순
            SortField::Relevance => Primary::None,
            SortField::CapturedAt => Primary::I64(ff.column_opt::<i64>("captured_at")?),
            SortField::CapturedOrModified => {
                Primary::I64(ff.column_opt::<i64>("captured_or_modified")?)
            }
            SortField::ModifiedAt => Primary::I64(ff.column_opt::<i64>("modified_at")?),
            SortField::Size => Primary::U64(ff.column_opt::<u64>("file_size")?),
            SortField::Duration => Primary::U64(ff.column_opt::<u64>("duration_ms")?),
            SortField::Rating => Primary::U64(ff.column_opt::<u64>("rating")?),
            SortField::Name => Primary::Name(
                ff.column_opt::<u64>("name_hi")?,
                ff.column_opt::<u64>("name_lo")?,
            ),
            SortField::Title => Primary::Name(
                ff.column_opt::<u64>("title_hi")?,
                ff.column_opt::<u64>("title_lo")?,
            ),
            SortField::CollectionOrder => match &self.order {
                Some(map) => Primary::Position(Arc::clone(map)),
                None => {
                    return Err(tantivy::TantivyError::InvalidArgument(
                        "collection order map missing".into(),
                    ));
                }
            },
        };
        Ok(KeyChild {
            asset_id: ff.u64("asset_id")?,
            primary,
            descending: self.sort.descending,
            after: self.after,
            invert: self.invert,
            skipped: Arc::clone(&self.skipped),
            skipped_local: 0,
        })
    }
}

// ---- 커서 ----

const CURSOR_PREFIX: &str = "c1";

pub(crate) fn encode_cursor(generation: u64, print: u64, key: &RankKey) -> String {
    format!(
        "{CURSOR_PREFIX}.{generation:x}.{print:x}.{:x}.{:x}.{:x}.{:x}",
        key[0], key[1], key[2], key[3]
    )
}

/// 형식 오류는 `InvalidInput`, 세대·요청 지문 불일치는 `StaleCursor`.
pub(crate) fn decode_cursor(cursor: &str, generation: u64, print: u64) -> Result<RankKey> {
    let bad = || SearchError::InvalidInput("malformed cursor".into());
    let mut parts = cursor.split('.');
    if parts.next() != Some(CURSOR_PREFIX) {
        return Err(bad());
    }
    let mut next = || -> Result<u64> {
        let p = parts.next().ok_or_else(bad)?;
        u64::from_str_radix(p, 16).map_err(|_| bad())
    };
    let (g, p) = (next()?, next()?);
    let key = [next()?, next()?, next()?, next()?];
    if parts.next().is_some() || key[3] == 0 {
        return Err(bad());
    }
    if g != generation || p != print {
        return Err(SearchError::StaleCursor);
    }
    Ok(key)
}

/// 질의·필터·정렬·전략의 지문 (FNV-1a). 같은 바이너리 안에서만 비교한다.
pub(crate) fn fingerprint(req: &CoordRequest) -> u64 {
    let text = format!(
        "{}|{:?}|{:?}|{:?}|{}",
        req.text.trim(),
        req.strategy,
        req.filters,
        req.sort.field,
        req.sort.descending
    );
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}
