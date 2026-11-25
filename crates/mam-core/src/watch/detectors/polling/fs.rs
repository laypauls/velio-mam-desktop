//! 폴링이 파일시스템을 보는 창구 — 시험에서는 가짜를 꽂고, 네트워크·이동식 볼륨은 `io_guard` 로 감싼다.
//! 읽기 전용 호출(디렉터리 수정 시각·목록)만 있다 — 원본에 쓰는 호출은 없다.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::stop::StopSignal;
use crate::scan::io_guard::IoGuard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollError {
    /// 제한 시간을 넘겼다(또는 볼륨이 응답 없음 상태) — "지금 알 수 없음" 이지 "없음" 이 아니다
    TimedOut,
    NotFound,
    /// 감지기 중지 요청으로 중단했다
    Stopped,
    Io(io::ErrorKind),
}

impl From<io::Error> for PollError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => Self::NotFound,
            k => Self::Io(k),
        }
    }
}

/// 디렉터리 하나의 직계 자식 이름 (심볼릭 링크·정션·유니코드가 아닌 이름은 제외한다).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub files: Vec<String>,
    pub dirs: Vec<String>,
}

pub trait PollFs: Send + Sync {
    /// 디렉터리 수정 시각 (epoch 이후 ns). 자식 추가·삭제·이름 변경이 이 값을 바꾼다 — 자식 파일의 내용만 바뀐 경우는 바꾸지 않는다.
    fn dir_mtime(&self, dir: &Path) -> Result<i64, PollError>;
    fn list_dir(&self, dir: &Path) -> Result<Listing, PollError>;
}

pub struct StdFs;

impl PollFs for StdFs {
    fn dir_mtime(&self, dir: &Path) -> Result<i64, PollError> {
        let t = std::fs::metadata(dir)?.modified()?;
        Ok(t.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .unwrap_or(0))
    }

    fn list_dir(&self, dir: &Path) -> Result<Listing, PollError> {
        let mut out = Listing::default();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let ft = entry.file_type()?;
            let Ok(name) = entry.file_name().into_string() else {
                continue; // 짝 없는 서로게이트 등 — 경로로 되돌릴 수 없다 (주기 재스캔이 맡는다)
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                out.dirs.push(name);
            } else if ft.is_file() {
                out.files.push(name);
            }
        }
        Ok(out)
    }
}

/// 막힐 수 있는 호출(NAS·이동식)에 제한 시간을 건다. 같은 경로의 호출은 `io_guard` 가 하나로 합치고, 볼륨별 상한을 지킨다.
/// 기다리는 동안에도 중지 요청을 짧은 간격으로 확인한다 — 막힌 공유 때문에 감지기 해제가 늦어지지 않게 한다.
pub struct GuardedFs {
    pub inner: Arc<dyn PollFs>,
    pub guard: Arc<IoGuard>,
    /// 검사 스레드 상한을 세는 단위 — 볼륨 ID 를 모르므로 폴더 ID 를 쓴다
    pub lane: i64,
    pub timeout: Duration,
    pub slice: Duration,
    pub stop: Arc<StopSignal>,
}

impl GuardedFs {
    fn call<T>(
        &self,
        key: String,
        f: impl Fn(&dyn PollFs) -> Result<T, PollError> + Clone + Send + 'static,
    ) -> Result<T, PollError>
    where
        T: Clone + Send + 'static,
    {
        let started = Instant::now();
        loop {
            if self.stop.is_set() {
                return Err(PollError::Stopped);
            }
            let (inner, f) = (Arc::clone(&self.inner), f.clone());
            let slice_started = Instant::now();
            let slice = self.slice.min(
                self.timeout
                    .saturating_sub(started.elapsed())
                    .max(Duration::from_millis(1)),
            );
            match self
                .guard
                .guarded(&key, self.lane, slice, move || f(&*inner))
            {
                crate::scan::io_guard::Guarded::Done(r) => return r,
                crate::scan::io_guard::Guarded::TimedOut => {
                    // 상한 때문에 걸지 않았다면(곧바로 돌아옴) 더 기다려도 소용없다
                    if slice_started.elapsed() < slice / 2 || started.elapsed() >= self.timeout {
                        return Err(PollError::TimedOut);
                    }
                }
            }
        }
    }
}

impl PollFs for GuardedFs {
    fn dir_mtime(&self, dir: &Path) -> Result<i64, PollError> {
        let p: PathBuf = dir.to_path_buf();
        self.call(format!("poll:mtime:{}", dir.display()), move |fs| {
            fs.dir_mtime(&p)
        })
    }

    fn list_dir(&self, dir: &Path) -> Result<Listing, PollError> {
        let p: PathBuf = dir.to_path_buf();
        self.call(format!("poll:list:{}", dir.display()), move |fs| {
            fs.list_dir(&p)
        })
    }
}
