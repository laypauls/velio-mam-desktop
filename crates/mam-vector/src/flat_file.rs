//! 전수 비교 인덱스의 파일 형식 — 64바이트 헤더 + ids(u64, 오름차순) + [scales(f32), i8 만] + 데이터(64바이트 정렬).
//!
//! 모든 구역이 8바이트 이상으로 정렬돼 있어 메모리 매핑한 바이트를 그대로 타입 슬라이스로 쓴다.
//! 파일 길이가 계산값과 정확히 같아야 열린다 — 잘린 파일은 [`VectorError::Corrupt`] 다.

use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::error::{Result, VectorError};
use crate::types::Quantization;

const MAGIC: &[u8; 8] = b"MAMFLAT\x01";
pub const HEADER_LEN: usize = 64;

pub fn quant_code(q: Quantization) -> u32 {
    match q {
        Quantization::F32 => 0,
        Quantization::F16 => 1,
        Quantization::I8 => 2,
    }
}

fn quant_from_code(c: u32) -> Option<Quantization> {
    match c {
        0 => Some(Quantization::F32),
        1 => Some(Quantization::F16),
        2 => Some(Quantization::I8),
        _ => None,
    }
}

pub fn elem_size(q: Quantization) -> usize {
    match q {
        Quantization::F32 => 4,
        Quantization::F16 => 2,
        Quantization::I8 => 1,
    }
}

/// 구역 오프셋과 전체 길이.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub n: usize,
    pub dim: usize,
    pub quant: Quantization,
    pub ids_off: usize,
    pub scales_off: usize,
    pub data_off: usize,
    pub total: usize,
}

fn align64(x: usize) -> usize {
    x.div_ceil(64) * 64
}

impl Layout {
    pub fn new(n: usize, dim: usize, quant: Quantization) -> Self {
        let ids_off = HEADER_LEN;
        let scales_off = ids_off + n * 8;
        let scales_len = if quant == Quantization::I8 { n * 4 } else { 0 };
        let data_off = align64(scales_off + scales_len);
        let total = data_off + n * dim * elem_size(quant);
        Self {
            n,
            dim,
            quant,
            ids_off,
            scales_off,
            data_off,
            total,
        }
    }
}

fn header(l: &Layout) -> [u8; HEADER_LEN] {
    let mut h = [0u8; HEADER_LEN];
    h[..8].copy_from_slice(MAGIC);
    h[8..12].copy_from_slice(&1u32.to_le_bytes()); // 형식 버전
    h[12..16].copy_from_slice(&(l.dim as u32).to_le_bytes());
    h[16..20].copy_from_slice(&quant_code(l.quant).to_le_bytes());
    h[24..32].copy_from_slice(&(l.n as u64).to_le_bytes());
    h
}

/// 헤더를 읽어 [`Layout`] 을 얻는다. 매직·버전·코드·길이가 하나라도 어긋나면 `Corrupt`.
pub fn parse_header(head: &[u8], file_len: u64) -> Result<Layout> {
    let bad = |m: &str| VectorError::Corrupt(format!("flat index: {m}"));
    if head.len() < HEADER_LEN || &head[..8] != MAGIC {
        return Err(bad("bad magic or truncated header"));
    }
    let u32_at = |o: usize| u32::from_le_bytes(head[o..o + 4].try_into().unwrap());
    if u32_at(8) != 1 {
        return Err(bad("unsupported format version"));
    }
    let dim = u32_at(12) as usize;
    let quant = quant_from_code(u32_at(16)).ok_or_else(|| bad("unknown quantization"))?;
    let n = u64::from_le_bytes(head[24..32].try_into().unwrap());
    // n·dim 곱이 현실적인 범위를 넘으면 손상으로 본다 (오버플로 방지)
    if dim == 0 || n > (1 << 40) || dim > (1 << 16) {
        return Err(bad("implausible dimension or count"));
    }
    let l = Layout::new(n as usize, dim, quant);
    if l.total as u64 != file_len {
        return Err(bad(&format!(
            "file length {file_len} != expected {} (truncated or extra data)",
            l.total
        )));
    }
    Ok(l)
}

/// 슬롯 순서대로 저장된 벡터 데이터 (정밀도별 타입).
pub enum DataVec {
    F32(Vec<f32>),
    F16(Vec<u16>),
    I8(Vec<i8>),
}

impl DataVec {
    pub fn with_capacity(q: Quantization, elems: usize) -> Self {
        match q {
            Quantization::F32 => Self::F32(Vec::with_capacity(elems)),
            Quantization::F16 => Self::F16(Vec::with_capacity(elems)),
            Quantization::I8 => Self::I8(Vec::with_capacity(elems)),
        }
    }

    fn row_bytes(&self, slot: usize, dim: usize) -> &[u8] {
        match self {
            Self::F32(v) => bytemuck::cast_slice(&v[slot * dim..(slot + 1) * dim]),
            Self::F16(v) => bytemuck::cast_slice(&v[slot * dim..(slot + 1) * dim]),
            Self::I8(v) => bytemuck::cast_slice(&v[slot * dim..(slot + 1) * dim]),
        }
    }
}

/// `order` 는 id 오름차순이 되는 슬롯 순서. 임시 파일에 쓰고 이름을 바꿔 중간 상태가 남지 않게 한다.
pub fn write(
    path: &Path,
    layout: &Layout,
    ids: &[u64],
    scales: &[f32],
    data: &DataVec,
    order: &[usize],
) -> Result<()> {
    debug_assert_eq!(order.len(), layout.n);
    let tmp = path.with_extension("tmp");
    {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
        w.write_all(&header(layout))?;
        for &s in order {
            w.write_all(&ids[s].to_le_bytes())?;
        }
        if layout.quant == Quantization::I8 {
            for &s in order {
                w.write_all(&scales[s].to_le_bytes())?;
            }
        }
        let pad = layout.data_off - (layout.scales_off + scales_len(layout));
        w.write_all(&vec![0u8; pad])?;
        for &s in order {
            w.write_all(data.row_bytes(s, layout.dim))?;
        }
        w.flush()?;
        w.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn scales_len(l: &Layout) -> usize {
    if l.quant == Quantization::I8 {
        l.n * 4
    } else {
        0
    }
}

/// 구역을 순서대로 읽어 RAM 에 올린다 (load).
pub struct Loaded {
    pub layout: Layout,
    pub ids: Vec<u64>,
    pub scales: Vec<f32>,
    pub data: DataVec,
}

pub fn read_all(path: &Path) -> Result<Loaded> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut head = [0u8; HEADER_LEN];
    f.read_exact(&mut head)
        .map_err(|_| VectorError::Corrupt("flat index: truncated header".into()))?;
    let layout = parse_header(&head, len)?;
    let mut ids = vec![0u64; layout.n];
    f.read_exact(bytemuck::cast_slice_mut(&mut ids))?;
    check_sorted(&ids)?;
    let mut scales = vec![
        0f32;
        if layout.quant == Quantization::I8 {
            layout.n
        } else {
            0
        }
    ];
    if !scales.is_empty() {
        f.read_exact(bytemuck::cast_slice_mut(&mut scales))?;
    }
    f.seek(SeekFrom::Start(layout.data_off as u64))?;
    let elems = layout.n * layout.dim;
    let data = match layout.quant {
        Quantization::F32 => {
            let mut v = vec![0f32; elems];
            f.read_exact(bytemuck::cast_slice_mut(&mut v))?;
            DataVec::F32(v)
        }
        Quantization::F16 => {
            let mut v = vec![0u16; elems];
            f.read_exact(bytemuck::cast_slice_mut(&mut v))?;
            DataVec::F16(v)
        }
        Quantization::I8 => {
            let mut v = vec![0i8; elems];
            f.read_exact(bytemuck::cast_slice_mut(&mut v))?;
            DataVec::I8(v)
        }
    };
    Ok(Loaded {
        layout,
        ids,
        scales,
        data,
    })
}

/// 이진 탐색이 가능하도록 ids 가 엄격히 오름차순인지 확인한다.
pub fn check_sorted(ids: &[u64]) -> Result<()> {
    if ids.windows(2).any(|w| w[0] >= w[1]) {
        return Err(VectorError::Corrupt(
            "flat index: ids are not strictly ascending".into(),
        ));
    }
    Ok(())
}
