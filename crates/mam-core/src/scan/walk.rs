//! 디렉터리 1개의 목록화와 제외 규칙, pre-order 비교 (재개 지점 판단).

use std::cmp::Ordering;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use mam_media::format::{self, MediaKind};

use super::media_types::MediaTypeSet;
use crate::reconcile::MediaType;
use crate::volume::strip_verbatim;

/// 자동 제외되는 폴더 이름 (대소문자 무시) — 휴지통·시스템 폴더 (D3), 볼륨 루트 메타 폴더
const RESERVED_DIRS: [&str; 3] = ["$recycle.bin", "system volume information", "$extend"];

#[cfg(windows)]
const ATTR_HIDDEN_SYSTEM: u32 = 0x2 | 0x4;

/// 경로 비교용 정규형: 확장 접두 제거, 구분자 통일, 소문자, 끝 구분자 제거.
fn norm(p: &Path) -> String {
    strip_verbatim(&p.to_string_lossy())
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

/// 절대 경로 기준 제외 목록 (MAMData·백업 폴더·사용자 제외 폴더).
#[derive(Debug, Clone, Default)]
pub struct WalkRules {
    excluded: HashSet<String>,
    /// 스캔 대상 유형 — 밖의 유형은 `Listing::skipped` 로 분리된다 (기본: 전부)
    types: MediaTypeSet,
}

impl WalkRules {
    pub fn new(excluded: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            excluded: excluded.into_iter().map(|p| norm(&p)).collect(),
            types: MediaTypeSet::ALL,
        }
    }

    pub fn with_types(mut self, types: MediaTypeSet) -> Self {
        self.types = types;
        self
    }

    pub fn types(&self) -> MediaTypeSet {
        self.types
    }

    pub fn is_excluded(&self, abs: &Path) -> bool {
        !self.excluded.is_empty() && self.excluded.contains(&norm(abs))
    }

    /// `abs` 가 제외 폴더 자신이거나 그 하위인가 (감시 이벤트 경로 검사용)
    pub fn is_under_excluded(&self, abs: &Path) -> bool {
        let n = norm(abs);
        self.excluded
            .iter()
            .any(|e| n == *e || n.starts_with(&format!("{e}\\")))
    }
}

#[derive(Debug, Clone)]
pub struct FileItem {
    pub name: String,
    pub key: String,
    pub abs: PathBuf,
    pub media: MediaType,
}

#[derive(Debug, Clone)]
pub struct DirItem {
    pub name: String,
    pub key: String,
    pub abs: PathBuf,
}

/// 읽는 도중 개별 항목에서 난 오류 (항목 이름 포함)
#[derive(Debug)]
pub struct EntryError {
    pub name: String,
    pub error: io::Error,
}

// 제한 시간 호출(io_guard)의 결과를 스레드 사이에서 나눠 쓰려면 복제할 수 있어야 한다 — `io::Error` 는 종류·메시지로 다시 만든다
impl Clone for EntryError {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            error: io::Error::new(self.error.kind(), self.error.to_string()),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Listing {
    pub files: Vec<FileItem>,
    /// 미디어 파일이지만 폴더의 대상 유형 밖인 것 — 등록하지 않고 존재 확인("봤다")만 한다
    pub skipped: Vec<FileItem>,
    pub dirs: Vec<DirItem>,
    pub entry_errors: Vec<EntryError>,
}

fn name_key(name: &str) -> String {
    mam_db::path_key(name).unwrap_or_else(|_| name.to_lowercase())
}

fn media_of(kind: MediaKind) -> MediaType {
    match kind {
        MediaKind::Image => MediaType::Image,
        MediaKind::Video => MediaType::Video,
        MediaKind::Audio => MediaType::Audio,
    }
}

/// 볼륨 루트의 시스템 폴더 이름인가 (`$RECYCLE.BIN`, `System Volume Information`, `$Extend` — 대소문자 무시).
/// 스캔(`list_dir`)과 감시 이벤트 경로(`apply_path`)가 같은 규칙을 쓴다.
pub fn is_reserved_dir_name(name: &str) -> bool {
    RESERVED_DIRS.contains(&name.to_lowercase().as_str())
}

/// 숨김 또는 시스템 속성이 붙은 디렉터리인가 (Windows 전용 — 다른 OS 는 항상 false). 읽을 수 없으면 false.
/// `list_dir` 의 기존 규칙(둘 중 하나라도 있으면 제외)과 같다 — 감시 경로도 같은 규칙으로 맞춘다.
pub fn is_hidden_or_system_dir(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        std::fs::symlink_metadata(path).is_ok_and(|m| {
            m.is_dir() && m.file_attributes() & ATTR_HIDDEN_SYSTEM != 0
        })
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

/// 숨김 항목(`.` 시작)인가
pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

/// 확장자로 미디어 종류를 정한다. 확장자가 없으면 앞부분 매직 바이트로 판별한다 (D4: 애매할 때만 내용을 읽는다).
pub fn classify_file(path: &Path) -> io::Result<Option<MediaType>> {
    match path.extension() {
        Some(ext) => Ok(format::kind_by_extension(&ext.to_string_lossy()).map(media_of)),
        None => match format::detect(path) {
            Ok(d) => Ok(d.kind.map(media_of)),
            Err(mam_media::MediaError::Io { source, .. }) => Err(source),
            Err(_) => Ok(None),
        },
    }
}

/// 디렉터리 하나를 읽어 대상 파일과 하위 디렉터리를 이름 키 순으로 돌려준다.
/// 읽기 자체가 실패하면 `Err`, 개별 항목 오류는 `entry_errors` 에 모은다. 심볼릭 링크·정션은 따라가지 않는다.
pub fn list_dir(dir: &Path, rules: &WalkRules) -> io::Result<Listing> {
    let mut out = Listing::default();
    for entry in std::fs::read_dir(dir)? {
        let entry = match entry {
            Ok(e) => e,
            Err(error) => {
                out.entry_errors.push(EntryError {
                    name: String::new(),
                    error,
                });
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_hidden_name(&name) {
            continue;
        }
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(error) => {
                out.entry_errors.push(EntryError { name, error });
                continue;
            }
        };
        if ft.is_symlink() {
            continue;
        }
        let abs = entry.path();
        if ft.is_dir() {
            if is_reserved_dir_name(&name) || rules.is_excluded(&abs) {
                continue;
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if let Ok(m) = entry.metadata()
                    && m.file_attributes() & ATTR_HIDDEN_SYSTEM != 0
                {
                    continue;
                }
            }
            out.dirs.push(DirItem {
                key: name_key(&name),
                name,
                abs,
            });
        } else if ft.is_file() {
            match classify_file(&abs) {
                Ok(Some(media)) => {
                    let item = FileItem {
                        key: name_key(&name),
                        name,
                        abs,
                        media,
                    };
                    if rules.types.contains(media) {
                        out.files.push(item);
                    } else {
                        out.skipped.push(item);
                    }
                }
                Ok(None) => {}
                Err(error) => out.entry_errors.push(EntryError { name, error }),
            }
        }
    }
    out.files
        .sort_by(|a, b| (&a.key, &a.name).cmp(&(&b.key, &b.name)));
    out.dirs
        .sort_by(|a, b| (&a.key, &a.name).cmp(&(&b.key, &b.name)));
    Ok(out)
}

fn segments(key: &str) -> impl Iterator<Item = &str> {
    key.split('/').filter(|s| !s.is_empty())
}

/// 폴더 루트 기준 디렉터리 키 두 개를 순회(pre-order) 순서로 비교한다. 부모는 자식보다 앞선다.
/// `''` 는 루트.
pub fn preorder_cmp(a: &str, b: &str) -> Ordering {
    segments(a).cmp(segments(b))
}

/// `dir` 하위 전체가 cursor 보다 앞서 이미 끝났는가 (cursor 가 그 안에 있으면 false — 내려가서 이어가야 한다).
pub fn subtree_before(dir: &str, cursor: &str) -> bool {
    preorder_cmp(dir, cursor) == Ordering::Less && !is_ancestor(dir, cursor)
}

/// `dir` 이 `other` 의 조상(또는 같음)인가
fn is_ancestor(dir: &str, other: &str) -> bool {
    let mut o = segments(other);
    segments(dir).all(|s| o.next() == Some(s))
}

#[cfg(test)]
mod tests {
    //! 분류: Business rule (제외·순서) / Boundary (루트, 형제 접두)
    use super::*;

    #[test]
    fn t_scan_walk_01_preorder_orders_parent_before_children_and_siblings_by_name() {
        assert_eq!(preorder_cmp("", "a"), Ordering::Less);
        assert_eq!(preorder_cmp("a", "a/b"), Ordering::Less);
        assert_eq!(
            preorder_cmp("a/z", "b"),
            Ordering::Less,
            "whole a-subtree precedes b"
        );
        assert_eq!(
            preorder_cmp("a-b", "a/b"),
            Ordering::Greater,
            "segment-wise, not byte-wise"
        );
        assert_eq!(preorder_cmp("a/b", "a/b"), Ordering::Equal);
    }

    #[test]
    fn t_scan_walk_02_subtree_before_only_skips_finished_siblings() {
        let cursor = "b/c";
        assert!(subtree_before("a", cursor));
        assert!(subtree_before("b/a", cursor));
        assert!(
            !subtree_before("b", cursor),
            "ancestor of the cursor must be entered"
        );
        assert!(
            !subtree_before("b/c", cursor),
            "cursor dir itself is entered for its children"
        );
        assert!(!subtree_before("b/d", cursor));
        assert!(!subtree_before("c", cursor));
    }

    #[test]
    fn t_scan_walk_03_rules_match_case_insensitively_and_by_prefix() {
        let rules = WalkRules::new([PathBuf::from(r"C:\Data\MAMData")]);
        assert!(rules.is_excluded(Path::new(r"c:\data\mamdata")));
        assert!(rules.is_excluded(Path::new(r"\\?\C:\Data\MAMData\")));
        assert!(!rules.is_excluded(Path::new(r"C:\Data\MAMData2")));
        assert!(rules.is_under_excluded(Path::new(r"C:\Data\MAMData\backups\x.jpg")));
        assert!(!rules.is_under_excluded(Path::new(r"C:\Data\MAMData2\x.jpg")));
    }

    #[test]
    fn t_scan_walk_04_list_dir_skips_excluded_dirs_inside_a_managed_tree_defense_in_depth() {
        // Arrange: 등록 단계(D2)가 이제 겹침을 거부하지만, MAM_DATA_DIR 환경 변수·기존 설치처럼 겹친 상태가 남아 있어도
        // 스캔은 MAMData·백업 폴더를 건너뛰어야 한다 (PL8 방어선)
        let dir = tempfile::TempDir::new().unwrap();
        let lib = dir.path().join("lib");
        for sub in ["MAMData", "my-backups", "ok"] {
            std::fs::create_dir_all(lib.join(sub)).unwrap();
        }
        let rules = WalkRules::new([lib.join("MAMData"), lib.join("my-backups")]);

        // Act
        let listing = list_dir(&lib, &rules).unwrap();

        // Assert
        let names: Vec<&str> = listing.dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["ok"]);
    }
}
