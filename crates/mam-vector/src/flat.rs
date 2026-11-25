//! 전수 비교(flat scan) 인덱스 — 엔진 B. 기준선이자 Recall 정답 계산기.
//!
//! - 저장 정밀도 f32 / f16 / i8(벡터별 스케일). 질의 시 전부 훑으므로 Recall 은 양자화 오차만큼만 손해다
//! - 메모리: RAM 적재([`FlatIndex::open`] `Load`) 또는 읽기 전용 메모리 매핑(`View`)
//! - 삭제는 swap-remove, 필터는 검색 중 판정(F1 과 동일)하거나 허용 id 집합만 훑는다([`FlatIndex::search_subset`], F3)

use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

use memmap2::Mmap;

use crate::error::{Result, VectorError};
use crate::flat_file::{self, DataVec, HEADER_LEN, Layout};
use crate::kernel::quantize_i8;
use crate::scan::{Data, Slices, default_threads, par_topk, run_subset};
use crate::types::{
    AnnIndex, EmbeddingId, Filter, Hit, OpenMode, Quantization, SearchParams, normalized,
    validate_query, validate_vector,
};

struct Owned {
    ids: Vec<u64>,
    scales: Vec<f32>,
    data: DataVec,
    slot: HashMap<u64, usize>,
}

struct Mapped {
    map: Mmap,
    layout: Layout,
}

enum Inner {
    Owned(Owned),
    Mapped(Mapped),
}

impl Owned {
    fn slices(&self, dim: usize) -> Slices<'_> {
        Slices {
            ids: &self.ids,
            scales: &self.scales,
            data: match &self.data {
                DataVec::F32(v) => Data::F32(v),
                DataVec::F16(v) => Data::F16(v),
                DataVec::I8(v) => Data::I8(v),
            },
            dim,
        }
    }
}

impl Mapped {
    fn slices(&self) -> Result<Slices<'_>> {
        let l = &self.layout;
        let b = &self.map[..];
        let cast_err = |e: bytemuck::PodCastError| VectorError::Corrupt(format!("flat index: {e}"));
        let ids = bytemuck::try_cast_slice(&b[l.ids_off..l.ids_off + l.n * 8]).map_err(cast_err)?;
        let scales = if l.quant == Quantization::I8 {
            bytemuck::try_cast_slice(&b[l.scales_off..l.scales_off + l.n * 4]).map_err(cast_err)?
        } else {
            &[][..]
        };
        let data_bytes = &b[l.data_off..];
        let data = match l.quant {
            Quantization::F32 => Data::F32(bytemuck::try_cast_slice(data_bytes).map_err(cast_err)?),
            Quantization::F16 => Data::F16(bytemuck::try_cast_slice(data_bytes).map_err(cast_err)?),
            Quantization::I8 => Data::I8(bytemuck::try_cast_slice(data_bytes).map_err(cast_err)?),
        };
        Ok(Slices {
            ids,
            scales,
            data,
            dim: l.dim,
        })
    }
}

pub struct FlatIndex {
    dim: usize,
    quant: Quantization,
    threads: AtomicUsize,
    inner: RwLock<Inner>,
}

impl FlatIndex {
    pub fn new(dim: usize, quant: Quantization) -> Self {
        Self::with_capacity(dim, quant, 0)
    }

    /// 대량 적재 시 재할당(피크 메모리 2배)을 피하려고 건수를 미리 예약한다.
    pub fn with_capacity(dim: usize, quant: Quantization, n: usize) -> Self {
        Self {
            dim,
            quant,
            threads: AtomicUsize::new(default_threads()),
            inner: RwLock::new(Inner::Owned(Owned {
                ids: Vec::with_capacity(n),
                scales: Vec::with_capacity(if quant == Quantization::I8 { n } else { 0 }),
                data: DataVec::with_capacity(quant, n * dim),
                slot: HashMap::with_capacity(n),
            })),
        }
    }

    /// 검색에 쓸 스레드 수 (측정에서 1 로 두면 단일 스레드 지연을 얻는다).
    pub fn set_threads(&self, n: usize) {
        self.threads.store(n.max(1), Ordering::Relaxed);
    }

    fn threads(&self) -> usize {
        self.threads.load(Ordering::Relaxed)
    }

    /// 파일을 연다. 헤더의 차원·정밀도가 기대값과 다르면 `IndexMismatch`, 길이·정렬이 어긋나면 `Corrupt`.
    pub fn open(path: &Path, dim: usize, quant: Quantization, mode: OpenMode) -> Result<Self> {
        let check = |l: &Layout| -> Result<()> {
            if l.dim != dim || l.quant != quant {
                return Err(VectorError::IndexMismatch(format!(
                    "flat file is dim={} {} but expected dim={dim} {}",
                    l.dim,
                    l.quant.as_str(),
                    quant.as_str()
                )));
            }
            Ok(())
        };
        let inner = match mode {
            OpenMode::Load => {
                let l = flat_file::read_all(path)?;
                check(&l.layout)?;
                let slot = l.ids.iter().enumerate().map(|(i, &id)| (id, i)).collect();
                Inner::Owned(Owned {
                    ids: l.ids,
                    scales: l.scales,
                    data: l.data,
                    slot,
                })
            }
            OpenMode::View => {
                let f = File::open(path)?;
                let len = f.metadata()?.len();
                if len < HEADER_LEN as u64 {
                    return Err(VectorError::Corrupt("flat index: truncated header".into()));
                }
                // SAFETY: 파일을 읽기 전용으로 매핑한다. 다른 프로세스가 같은 파일을 줄이면 접근 오류가
                // 날 수 있으나, 인덱스 디렉터리는 이 앱만 쓰며 저장은 새 파일 + rename 으로만 한다.
                let map = unsafe { Mmap::map(&f)? };
                let layout = flat_file::parse_header(&map[..HEADER_LEN], len)?;
                check(&layout)?;
                let m = Mapped { map, layout };
                flat_file::check_sorted(m.slices()?.ids)?;
                Inner::Mapped(m)
            }
        };
        Ok(Self {
            dim,
            quant,
            threads: AtomicUsize::new(default_threads()),
            inner: RwLock::new(inner),
        })
    }

    pub fn is_view(&self) -> bool {
        matches!(*self.inner.read().unwrap(), Inner::Mapped(_))
    }

    pub fn contains(&self, id: EmbeddingId) -> bool {
        match &*self.inner.read().unwrap() {
            Inner::Owned(o) => o.slot.contains_key(&id),
            Inner::Mapped(m) => m.slices().map(|s| s.slot_of(id).is_some()).unwrap_or(false),
        }
    }

    /// 읽기 잠금 아래에서 슬라이스 뷰와 스레드 수로 `f` 를 실행한다 (`search_multi` 같은 확장 검색용).
    pub(crate) fn with_slices<R>(&self, f: impl FnOnce(&Slices<'_>, usize) -> R) -> Result<R> {
        let g = self.inner.read().unwrap();
        let threads = self.threads();
        Ok(match &*g {
            Inner::Owned(o) => f(&o.slices(self.dim), threads),
            Inner::Mapped(m) => f(&m.slices()?, threads),
        })
    }

    pub(crate) fn dim(&self) -> usize {
        self.dim
    }

    /// F3: 허용된 `embedding_id` 목록만 전수 비교한다. 인덱스에 없는 id 는 건너뛴다.
    pub fn search_subset(&self, q: &[f32], k: usize, allowed: &[EmbeddingId]) -> Result<Vec<Hit>> {
        validate_query(self.dim, q, k)?;
        let g = self.inner.read().unwrap();
        let threads = self.threads();
        match &*g {
            Inner::Owned(o) => {
                let s = o.slices(self.dim);
                let slots: Vec<usize> = allowed
                    .iter()
                    .filter_map(|id| o.slot.get(id).copied())
                    .collect();
                Ok(run_subset(&s, q, k, &slots, threads))
            }
            Inner::Mapped(m) => {
                let s = m.slices()?;
                let slots: Vec<usize> = allowed.iter().filter_map(|&id| s.slot_of(id)).collect();
                Ok(run_subset(&s, q, k, &slots, threads))
            }
        }
    }
}

impl AnnIndex for FlatIndex {
    fn engine_name(&self) -> &'static str {
        "flat"
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn quantization(&self) -> Quantization {
        self.quant
    }

    fn len(&self) -> usize {
        match &*self.inner.read().unwrap() {
            Inner::Owned(o) => o.ids.len(),
            Inner::Mapped(m) => m.layout.n,
        }
    }

    fn add(&self, id: EmbeddingId, v: &[f32]) -> Result<()> {
        validate_vector(self.dim, v)?;
        let vn = normalized(v);
        let mut g = self.inner.write().unwrap();
        let Inner::Owned(o) = &mut *g else {
            return Err(VectorError::ReadOnly);
        };
        let dim = self.dim;
        let (slot, fresh) = match o.slot.get(&id) {
            Some(&s) => (s, false),
            None => (o.ids.len(), true),
        };
        let r = slot * dim..(slot + 1) * dim;
        match &mut o.data {
            DataVec::F32(d) => {
                if fresh {
                    d.extend_from_slice(&vn);
                } else {
                    d[r].copy_from_slice(&vn);
                }
            }
            DataVec::F16(d) => {
                let bits = vn.iter().map(|&x| half::f16::from_f32(x).to_bits());
                if fresh {
                    d.extend(bits);
                } else {
                    d[r].iter_mut().zip(bits).for_each(|(o, b)| *o = b);
                }
            }
            DataVec::I8(d) => {
                let mut qv = vec![0i8; dim];
                let scale = quantize_i8(&vn, &mut qv);
                if fresh {
                    d.extend_from_slice(&qv);
                    o.scales.push(scale);
                } else {
                    d[r].copy_from_slice(&qv);
                    o.scales[slot] = scale;
                }
            }
        }
        if fresh {
            o.ids.push(id);
            o.slot.insert(id, slot);
        }
        Ok(())
    }

    fn remove(&self, id: EmbeddingId) -> Result<bool> {
        let mut g = self.inner.write().unwrap();
        let Inner::Owned(o) = &mut *g else {
            return Err(VectorError::ReadOnly);
        };
        let Some(slot) = o.slot.remove(&id) else {
            return Ok(false);
        };
        let (dim, last) = (self.dim, o.ids.len() - 1);
        if slot != last {
            // 마지막 행을 빈 자리로 옮긴다 (swap-remove)
            match &mut o.data {
                DataVec::F32(d) => d.copy_within(last * dim..(last + 1) * dim, slot * dim),
                DataVec::F16(d) => d.copy_within(last * dim..(last + 1) * dim, slot * dim),
                DataVec::I8(d) => {
                    d.copy_within(last * dim..(last + 1) * dim, slot * dim);
                    o.scales[slot] = o.scales[last];
                }
            }
            let moved = o.ids[last];
            o.ids[slot] = moved;
            o.slot.insert(moved, slot);
        }
        o.ids.truncate(last);
        match &mut o.data {
            DataVec::F32(d) => d.truncate(last * dim),
            DataVec::F16(d) => d.truncate(last * dim),
            DataVec::I8(d) => {
                d.truncate(last * dim);
                o.scales.truncate(last);
            }
        }
        Ok(true)
    }

    fn search(
        &self,
        q: &[f32],
        k: usize,
        _params: &SearchParams,
        filter: Option<Filter<'_>>,
    ) -> Result<Vec<Hit>> {
        validate_query(self.dim, q, k)?;
        let g = self.inner.read().unwrap();
        let threads = self.threads();
        let run = |s: &Slices<'_>| {
            let pq = s.prepare(q);
            par_topk(s.len(), threads, k, &|lo, hi| {
                s.scan_range(&pq, k, filter, lo, hi)
            })
            .into_sorted()
        };
        Ok(match &*g {
            Inner::Owned(o) => run(&o.slices(self.dim)),
            Inner::Mapped(m) => run(&m.slices()?),
        })
    }

    fn memory_bytes(&self) -> usize {
        match &*self.inner.read().unwrap() {
            Inner::Owned(o) => {
                o.ids.capacity() * 8
                    + o.scales.capacity() * 4
                    + o.slot.capacity() * 16
                    + match &o.data {
                        DataVec::F32(v) => v.capacity() * 4,
                        DataVec::F16(v) => v.capacity() * 2,
                        DataVec::I8(v) => v.capacity(),
                    }
            }
            Inner::Mapped(m) => m.layout.total,
        }
    }

    fn save_file(&self, path: &Path) -> Result<()> {
        let g = self.inner.read().unwrap();
        let Inner::Owned(o) = &*g else {
            return Err(VectorError::Unsupported("saving a memory-mapped view"));
        };
        let mut order: Vec<usize> = (0..o.ids.len()).collect();
        order.sort_unstable_by_key(|&s| o.ids[s]);
        let layout = Layout::new(o.ids.len(), self.dim, self.quant);
        flat_file::write(path, &layout, &o.ids, &o.scales, &o.data, &order)
    }
}
