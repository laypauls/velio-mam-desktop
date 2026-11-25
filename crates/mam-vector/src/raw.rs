//! 원시 임베딩 영속 저장 (D2) — 인덱스는 파생 데이터이므로 이것으로 재구축할 수 있어야 한다.
//!
//! - (a) SQLite BLOB: `poc_embedding_vectors.vector` (f16 리틀엔디언). 메타와 같은 트랜잭션에 커밋된다
//! - (b) 추가 전용 파일 `raw.f16` + SQLite 에는 `raw_offset` 만. 파일을 먼저 fsync 한 뒤 SQLite 를 커밋하므로
//!   비정상 종료 시 파일 꼬리에 고아 레코드가 남을 수는 있어도 SQLite 가 가리키는 레코드가 없을 수는 없다
//!
//! 마이그레이션이 아니라 **측정 DB 에만** 만드는 `poc_*` 테이블이다 (D3). BLOB 을 `poc_embeddings` 와
//! 같은 행에 두지 않고 곁 테이블로 분리한 이유: 메타 조회(필터 집합 구하기)가 큰 행을 건너뛰며 훑어야 하기 때문이다.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use memmap2::Mmap;
use rusqlite::{Connection, params};

use crate::error::{Result, VectorError};
use crate::types::EmbeddingId;

/// 측정 DB 에 만드는 스키마 (D3). `embedding_id` 는 `AUTOINCREMENT` — 재사용 금지.
pub const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS poc_embeddings (
    embedding_id  INTEGER PRIMARY KEY AUTOINCREMENT,
    asset_id      INTEGER NOT NULL REFERENCES media_assets (asset_id),
    segment_id    INTEGER,
    vector_kind   TEXT NOT NULL DEFAULT 'visual' CHECK (vector_kind IN ('visual', 'semantic')),
    model_id      TEXT NOT NULL,
    model_version TEXT NOT NULL,
    dimension     INTEGER NOT NULL CHECK (dimension > 0),
    raw_offset    INTEGER
) STRICT;
CREATE INDEX IF NOT EXISTS idx_poc_embeddings_asset ON poc_embeddings (asset_id);
CREATE TABLE IF NOT EXISTS poc_embedding_vectors (
    embedding_id INTEGER PRIMARY KEY REFERENCES poc_embeddings (embedding_id),
    vector       BLOB NOT NULL
) STRICT;
";

pub fn create_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA_SQL)?;
    Ok(())
}

/// 손상으로 잃은 레코드 자리를 메우는 바이트. f16 `0xFFFF` 는 NaN 이고 저장 전 검증이 NaN 을 막으므로
/// 정상 레코드의 첫 원소와 겹칠 수 없다.
pub(crate) const HOLE_BYTE: u8 = 0xFF;

/// 레코드가 [`HOLE_BYTE`] 채움(= 손상으로 잃은 자리)인가.
pub(crate) fn is_hole(rec: &[u8]) -> bool {
    rec.len() >= 2 && rec[0] == HOLE_BYTE && rec[1] == HOLE_BYTE
}

/// f32 벡터를 f16 리틀엔디언 바이트로 (저장 정밀도 손실은 이 변환에서만 생긴다).
pub fn f32_to_f16_bytes(v: &[f32]) -> Vec<u8> {
    let bits: Vec<u16> = v
        .iter()
        .map(|&x| half::f16::from_f32(x).to_bits().to_le())
        .collect();
    bytemuck::cast_slice(&bits).to_vec()
}

pub(crate) fn f16_bytes_to_f32(b: &[u8], out: &mut Vec<f32>) -> Result<()> {
    if !b.len().is_multiple_of(2) {
        return Err(VectorError::Corrupt("raw f16 record has odd length".into()));
    }
    out.clear();
    out.extend(
        b.chunks_exact(2)
            .map(|c| half::f16::from_bits(u16::from_le_bytes([c[0], c[1]])).to_f32()),
    );
    Ok(())
}

/// (a) 한 건 저장. 호출자가 트랜잭션을 연다.
pub fn put_blob(conn: &Connection, id: EmbeddingId, v: &[f32]) -> Result<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO poc_embedding_vectors (embedding_id, vector) VALUES (?1, ?2)",
    )?
    .execute(params![id as i64, f32_to_f16_bytes(v)])?;
    Ok(())
}

/// (a) 전체를 `embedding_id` 순으로 읽어 콜백에 넘긴다. 콜백 오류는 그대로 전파한다.
pub fn for_each_blob(
    conn: &Connection,
    dim: usize,
    mut f: impl FnMut(EmbeddingId, &[f32]) -> Result<()>,
) -> Result<u64> {
    let mut stmt = conn
        .prepare("SELECT embedding_id, vector FROM poc_embedding_vectors ORDER BY embedding_id")?;
    let mut rows = stmt.query([])?;
    let (mut buf, mut n) = (Vec::with_capacity(dim), 0u64);
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let blob: Vec<u8> = r.get(1)?;
        f16_bytes_to_f32(&blob, &mut buf)?;
        if buf.len() != dim {
            return Err(VectorError::DimensionMismatch {
                expected: dim,
                found: buf.len(),
            });
        }
        f(id as u64, &buf)?;
        n += 1;
    }
    Ok(n)
}

/// (b) 추가 전용 원시 파일. 레코드는 `dim × 2` 바이트 고정 길이라 `offset = 순번 × 레코드 길이`.
pub struct RawFile {
    path: PathBuf,
    file: File,
    dim: usize,
    len: u64,
}

impl RawFile {
    pub(crate) fn record_len(dim: usize) -> u64 {
        dim as u64 * 2
    }

    /// 없으면 만들고, 있으면 이어 쓴다. 길이가 레코드 길이의 배수가 아니면 중간에 끊긴 쓰기이므로
    /// 마지막 온전한 레코드까지 잘라 낸다 (그 꼬리를 가리키는 SQLite 행은 없다 — 파일을 먼저 쓰고 커밋하므로).
    pub fn open(path: &Path, dim: usize) -> Result<Self> {
        Self::open_with_floor(path, dim, 0)
    }

    /// [`Self::open`] + "SQLite 가 이미 가리킨 영역은 재사용하지 않는다" 보장.
    ///
    /// `referenced_end` 는 SQLite 행이 참조하는 가장 큰 `raw_offset + 레코드 길이`다. 파일이 그보다 짧으면
    /// (꼬리 손상·잘림) 그 사이를 [`HOLE_BYTE`] 로 채워 다음 `append` 가 `referenced_end` 이후에서 시작하게 한다.
    /// 그러지 않으면 손상된 행의 `raw_offset` 이 새 레코드와 겹쳐 다른 자산의 벡터가 조용히 읽힌다.
    /// 채움 구간은 [`is_hole`] 로 판별되어 읽기 쪽이 `skipped` 로 보고한다 (정상 벡터는 NaN 을 담지 않는다).
    /// 크래시 안전: 채움도 fsync 한 뒤에만 반환하며, 이후 레코드는 기존 규칙(파일 fsync -> SQLite 커밋)을 따른다.
    pub fn open_with_floor(path: &Path, dim: usize, referenced_end: u64) -> Result<Self> {
        let mut this = Self::open_inner(path, dim)?;
        this.ensure_floor(referenced_end)?;
        Ok(this)
    }

    /// 열린 뒤에 floor 를 적용한다 ([`Self::open_with_floor`] 의 두 번째 절반) — 호출자가 파일 길이를 보고
    /// floor 를 DB 에서 구할지 결정할 때 쓴다.
    pub(crate) fn ensure_floor(&mut self, referenced_end: u64) -> Result<()> {
        let rl = Self::record_len(self.dim);
        // 레코드 경계로 올림 (오프셋은 항상 배수지만 손상된 DB 값에도 안전하게)
        let floor = referenced_end.div_ceil(rl) * rl;
        if self.len < floor {
            let pad = vec![HOLE_BYTE; (floor - self.len) as usize];
            self.file.seek(SeekFrom::End(0))?;
            self.file.write_all(&pad)?;
            self.file.sync_all()?;
            self.len = floor;
        }
        Ok(())
    }

    fn open_inner(path: &Path, dim: usize) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            // append(true) 로 열면 Windows 에서 FILE_WRITE_DATA 가 빠져 set_len(꼬리 잘라내기)이 ACCESS_DENIED 가 된다 -> write + 직접 끝으로 seek
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let rl = Self::record_len(dim);
        let len = file.metadata()?.len();
        let whole = len - len % rl;
        if whole != len {
            file.set_len(whole)?;
        }
        Ok(Self {
            path: path.to_path_buf(),
            file,
            dim,
            len: whole,
        })
    }

    pub fn len_bytes(&self) -> u64 {
        self.len
    }

    /// 벡터들을 파일 끝에 추가하고 각 오프셋을 돌려준다. `sync` 가 끝난 뒤에만 SQLite 에 커밋할 것.
    pub fn append(&mut self, vectors: &[&[f32]]) -> Result<Vec<u64>> {
        let mut offsets = Vec::with_capacity(vectors.len());
        let mut buf = Vec::with_capacity(vectors.len() * self.dim * 2);
        for v in vectors {
            if v.len() != self.dim {
                return Err(VectorError::DimensionMismatch {
                    expected: self.dim,
                    found: v.len(),
                });
            }
            offsets.push(self.len + buf.len() as u64);
            buf.extend_from_slice(&f32_to_f16_bytes(v));
        }
        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&buf)?;
        self.len += buf.len() as u64;
        Ok(offsets)
    }

    pub fn sync(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// (b) `poc_embeddings.raw_offset` 순서(= `embedding_id` 순)로 파일을 읽어 콜백에 넘긴다.
/// SQLite 가 가리키는 오프셋이 파일 밖이면 `Corrupt` (파일이 SQLite 보다 앞서 쓰이므로 정상 종료에서는 일어나지 않는다).
pub fn for_each_file(
    conn: &Connection,
    raw_path: &Path,
    dim: usize,
    mut f: impl FnMut(EmbeddingId, &[f32]) -> Result<()>,
) -> Result<u64> {
    let file = File::open(raw_path)?;
    if file.metadata()?.len() == 0 {
        return Ok(0);
    }
    // SAFETY: 읽기 전용 매핑. 파일은 이 프로세스만 추가 쓰기하며 읽는 동안 줄어들지 않는다.
    let map = unsafe { Mmap::map(&file)? };
    let rl = RawFile::record_len(dim) as usize;
    let mut stmt = conn.prepare(
        "SELECT embedding_id, raw_offset FROM poc_embeddings WHERE raw_offset IS NOT NULL ORDER BY embedding_id",
    )?;
    let mut rows = stmt.query([])?;
    let (mut buf, mut n) = (Vec::with_capacity(dim), 0u64);
    while let Some(r) = rows.next()? {
        let id: i64 = r.get(0)?;
        let off: i64 = r.get(1)?;
        let off =
            usize::try_from(off).map_err(|_| VectorError::Corrupt("negative raw_offset".into()))?;
        let rec = map.get(off..off + rl).ok_or_else(|| {
            VectorError::Corrupt(format!("raw_offset {off} beyond file for embedding {id}"))
        })?;
        f16_bytes_to_f32(rec, &mut buf)?;
        f(id as u64, &buf)?;
        n += 1;
    }
    Ok(n)
}
