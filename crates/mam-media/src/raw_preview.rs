//! RAW 내장 JPEG 프리뷰 추출 (D3) — TIFF/EXIF 구조(DNG·CR2·NEF·ARW 등)를 걸으며 JPEG 후보를 찾는다.
//!
//! 전체 RAW 현상(LibRaw)은 하지 않는다. 파일은 읽기 전용으로 열고 IFD 와 선택한 프리뷰 구간만 읽는다.
//! 실제 카메라 RAW 로는 시험하지 못했다 — 합성 TIFF 구조 샘플로만 검증됨(작업 문서 참조).

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::error::{MediaError, Result};

const MAX_IFDS: usize = 32;
const MAX_ENTRIES: u16 = 512;
const MAX_PREVIEW_BYTES: u64 = 64 * 1024 * 1024;
const SCAN_BYTES: u64 = 128 * 1024;

const TAG_COMPRESSION: u16 = 0x0103;
const TAG_PHOTOMETRIC: u16 = 0x0106;
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_STRIP_COUNTS: u16 = 0x0117;
const TAG_SUB_IFDS: u16 = 0x014A;
const TAG_JPEG_OFFSET: u16 = 0x0201;
const TAG_JPEG_LENGTH: u16 = 0x0202;

struct Entry {
    tag: u16,
    typ: u16,
    count: u32,
    field: [u8; 4],
}

struct Tiff<R> {
    r: R,
    le: bool,
    file_len: u64,
}

impl<R: Read + Seek> Tiff<R> {
    fn u16(&self, b: [u8; 2]) -> u16 {
        if self.le {
            u16::from_le_bytes(b)
        } else {
            u16::from_be_bytes(b)
        }
    }
    fn u32(&self, b: [u8; 4]) -> u32 {
        if self.le {
            u32::from_le_bytes(b)
        } else {
            u32::from_be_bytes(b)
        }
    }

    fn read_at(&mut self, off: u64, len: usize) -> Option<Vec<u8>> {
        if off.checked_add(len as u64)? > self.file_len {
            return None;
        }
        self.r.seek(SeekFrom::Start(off)).ok()?;
        let mut buf = vec![0u8; len];
        self.r.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    /// IFD 를 읽어 (엔트리들, 다음 IFD 오프셋) 을 돌려준다.
    fn read_ifd(&mut self, off: u64) -> Option<(Vec<Entry>, u32)> {
        let b: [u8; 2] = self.read_at(off, 2)?.try_into().ok()?;
        let n = self.u16(b);
        if n == 0 || n > MAX_ENTRIES {
            return None;
        }
        let raw = self.read_at(off + 2, n as usize * 12 + 4)?;
        let entries = raw[..n as usize * 12]
            .chunks_exact(12)
            .map(|c| Entry {
                tag: self.u16([c[0], c[1]]),
                typ: self.u16([c[2], c[3]]),
                count: self.u32([c[4], c[5], c[6], c[7]]),
                field: [c[8], c[9], c[10], c[11]],
            })
            .collect();
        let tail = &raw[n as usize * 12..];
        Some((entries, self.u32([tail[0], tail[1], tail[2], tail[3]])))
    }

    /// SHORT/LONG 값들 (최대 `max` 개).
    fn values(&mut self, e: &Entry, max: u32) -> Option<Vec<u32>> {
        let size = match e.typ {
            3 => 2usize,
            4 => 4,
            _ => return None,
        };
        if e.count == 0 || e.count > max {
            return None;
        }
        let total = size * e.count as usize;
        let bytes = if total <= 4 {
            e.field[..total].to_vec()
        } else {
            self.read_at(self.u32(e.field) as u64, total)?
        };
        Some(
            bytes
                .chunks_exact(size)
                .map(|c| {
                    if size == 2 {
                        self.u16([c[0], c[1]]) as u32
                    } else {
                        self.u32([c[0], c[1], c[2], c[3]])
                    }
                })
                .collect(),
        )
    }
}

/// JPEG 의 (픽셀 수) — Baseline/Extended/Progressive Huffman 만 인정한다.
/// 무손실 JPEG(SOF3, DNG 의 실제 RAW 데이터)나 산술 부호화 등은 `None`.
fn jpeg_pixels(head: &[u8]) -> Option<u64> {
    if head.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2;
    loop {
        if *head.get(pos)? != 0xFF {
            return None;
        }
        while *head.get(pos)? == 0xFF {
            pos += 1;
        }
        let marker = *head.get(pos)?;
        pos += 1;
        match marker {
            0x01 | 0xD0..=0xD8 => continue,
            0xC0..=0xC2 => {
                let seg = head.get(pos..pos + 7)?;
                let h = u16::from_be_bytes([seg[3], seg[4]]) as u64;
                let w = u16::from_be_bytes([seg[5], seg[6]]) as u64;
                return (w > 0 && h > 0).then_some(w * h);
            }
            0xC3..=0xCF | 0xD9 | 0xDA => return None,
            _ => {
                let len = u16::from_be_bytes([*head.get(pos)?, *head.get(pos + 1)?]) as usize;
                if len < 2 {
                    return None;
                }
                pos += len;
            }
        }
    }
}

/// RAW(TIFF 구조) 파일에서 가장 큰 내장 JPEG 프리뷰 바이트를 꺼낸다.
pub fn extract_raw_preview(path: &Path) -> Result<Vec<u8>> {
    let mut file = File::open(path).map_err(|e| MediaError::io(path, e))?;
    let file_len = file.metadata().map_err(|e| MediaError::io(path, e))?.len();
    let mut hdr = [0u8; 8];
    file.read_exact(&mut hdr)
        .map_err(|_| MediaError::NoRawPreview)?;
    let le = match &hdr[..4] {
        b"II*\0" => true,
        b"MM\0*" => false,
        _ => return Err(MediaError::NoRawPreview),
    };
    let mut t = Tiff {
        r: file,
        le,
        file_len,
    };

    let mut queue = vec![t.u32([hdr[4], hdr[5], hdr[6], hdr[7]]) as u64];
    let mut seen = HashSet::new();
    let mut candidates: Vec<(u64, u64)> = Vec::new(); // (offset, len)
    while let Some(off) = queue.pop() {
        if off == 0 || !seen.insert(off) || seen.len() > MAX_IFDS {
            continue;
        }
        let Some((entries, next)) = t.read_ifd(off) else {
            continue;
        };
        let one = |t: &mut Tiff<File>, tag: u16| {
            entries
                .iter()
                .find(|e| e.tag == tag)
                .and_then(|e| t.values(e, 1))
                .map(|v| v[0])
        };
        let (mut jpeg_off, mut jpeg_len) = (None, None);
        for e in entries.iter().filter(|e| e.tag == TAG_SUB_IFDS) {
            if let Some(v) = t.values(e, 16) {
                queue.extend(v.into_iter().map(u64::from));
            }
        }
        if let (Some(o), Some(l)) = (one(&mut t, TAG_JPEG_OFFSET), one(&mut t, TAG_JPEG_LENGTH)) {
            (jpeg_off, jpeg_len) = (Some(o), Some(l));
        } else if matches!(one(&mut t, TAG_COMPRESSION), Some(6 | 7)) {
            // 스트립 방식 JPEG. CFA(32803)/LinearRaw(34892) 는 프리뷰가 아니라 RAW 데이터 자체다.
            let photometric = one(&mut t, TAG_PHOTOMETRIC);
            if !matches!(photometric, Some(32803 | 34892)) {
                (jpeg_off, jpeg_len) = (
                    one(&mut t, TAG_STRIP_OFFSETS),
                    one(&mut t, TAG_STRIP_COUNTS),
                );
            }
        }
        if let (Some(o), Some(l)) = (jpeg_off, jpeg_len) {
            candidates.push((o as u64, l as u64));
        }
        queue.push(next as u64);
    }

    let best = candidates
        .into_iter()
        .filter(|&(_, len)| len > 4 && len <= MAX_PREVIEW_BYTES)
        .filter_map(|(off, len)| {
            let head = t.read_at(off, len.min(SCAN_BYTES) as usize)?;
            Some((jpeg_pixels(&head)?, off, len))
        })
        .max_by_key(|&(px, _, len)| (px, len))
        .ok_or(MediaError::NoRawPreview)?;
    t.read_at(best.1, best.2 as usize)
        .ok_or(MediaError::NoRawPreview)
}
