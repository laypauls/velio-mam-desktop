//! 표준 파일 연산 구현. 원본(`src`)은 읽기 전용으로만 연다.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::ExportFs;

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct StdFs;

fn tmp_beside(dst: &Path) -> PathBuf {
    let n = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    dst.with_file_name(format!(".mam-export-{}-{n}.tmp", std::process::id()))
}

/// `create_new` 로 파일을 만들어 `fill` 로 채운다. 실패하면 만든 파일을 지운다 (이미 있던 파일은 건드리지 않는다).
fn create_filled(dst: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(dst)?;
    let result = fill(&mut file).and_then(|()| file.flush());
    drop(file);
    if result.is_err() {
        let _ = fs::remove_file(dst);
    }
    result
}

/// 임시 파일에 쓴 뒤 `dst` 로 rename 한다 — 덮어쓰기 중간 실패로 기존 파일이 반쯤 깨지지 않는다.
fn replace_filled(dst: &Path, fill: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let tmp = tmp_beside(dst);
    create_filled(&tmp, fill)?;
    fs::rename(&tmp, dst).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

impl ExportFs for StdFs {
    fn copy(&self, src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
        let mut input = File::open(src)?;
        let fill = |out: &mut File| io::copy(&mut input, out).map(|_| ());
        if replace {
            replace_filled(dst, fill)
        } else {
            create_filled(dst, fill)
        }
    }

    fn write(&self, dst: &Path, data: &[u8], replace: bool) -> io::Result<()> {
        let fill = |out: &mut File| out.write_all(data);
        if replace {
            replace_filled(dst, fill)
        } else {
            create_filled(dst, fill)
        }
    }
}
