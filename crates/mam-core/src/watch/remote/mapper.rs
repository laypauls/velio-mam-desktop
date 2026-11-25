//! 볼륨(스토리지) 경로 → 이 PC 경로 변환 규칙. watch_folder `ddpaudit/pathmap.go` + `config.ddpMapReaches` 의 규칙을 옮겼다.
//!
//! ```text
//! from: "/"          to: "F:\UHD"     /1TV/a.mxf  ->  F:\UHD\1TV\a.mxf
//! from: "/Protools"  to: "G:\프로툴"   /Protools/x ->  G:\프로툴\x
//! ```
//!
//! 감사 로그의 경로는 "볼륨 기준" 이고 어느 볼륨인지는 줄에 없다 — 규칙이 안 맞는 경로는 버리고 센다(`Collector` 의 `unmapped`).
//! 문자열로만 계산한다(Windows 경로 규칙: `\` 구분, 대소문자 무시) — 원격에서 온 **믿을 수 없는 문자열**이므로 `..` 과 윈도우 예약 문자는 거부한다.

use serde::{Deserialize, Serialize};

use crate::volume::strip_verbatim;

/// 규칙 한 줄. 저장 형식(`path_map_json`)과 같다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Default)]
pub struct Mapper {
    rules: Vec<Mapping>,
}

impl Mapper {
    /// 규칙을 긴 `from` 부터 맞춰 보도록 정렬한다 (같은 길이는 입력 순서). `to` 가 비면 규칙으로 치지 않는다.
    pub fn new(rules: &[Mapping]) -> Self {
        let mut v: Vec<Mapping> = rules
            .iter()
            .filter_map(|r| {
                Some(Mapping {
                    from: norm_from(&r.from),
                    to: clean_to(&r.to)?,
                })
            })
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.from.len())); // 안정 정렬
        Self { rules: v }
    }

    /// 볼륨 상대 경로를 이 PC 경로로 바꾼다. 맞는 규칙이 없거나 경로가 위험하면 `None`.
    pub fn map(&self, vol_path: &str) -> Option<String> {
        let p = norm_from(vol_path);
        if !p.split('/').all(safe_segment) {
            return None;
        }
        self.rules.iter().find_map(|r| {
            let rest = under(&p, &r.from)?;
            Some(if rest.is_empty() { r.to.clone() } else { join(&r.to, rest) })
        })
    }

    pub fn len(&self) -> usize {
        self.rules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// 세그먼트가 안전한가 — 빈 칸(루트)은 통과, `..` 와 윈도우 예약 문자·제어 문자는 거부.
fn safe_segment(seg: &str) -> bool {
    seg != ".." && !seg.chars().any(|c| c.is_control() || "<>:\"|?*".contains(c))
}

/// 앞에 `/` 를 붙이고 뒤의 `/` 를 떼어 `/a/b` 꼴로 맞춘다. 루트는 빈 문자열.
fn norm_from(s: &str) -> String {
    let t = s.trim().replace('\\', "/");
    let t = t.trim_matches('/');
    if t.is_empty() { String::new() } else { format!("/{t}") }
}

/// `p` 가 `base` 아래인지 한 칸 단위로 본다 (`/Proto` 가 `/Protools/x` 를 먹지 않는다). 아래면 `base` 를 뗀 나머지.
fn under<'a>(p: &'a str, base: &str) -> Option<&'a str> {
    if base.is_empty() {
        return Some(p.trim_start_matches('/'));
    }
    if p == base {
        return Some("");
    }
    p.strip_prefix(base)?.strip_prefix('/')
}

/// 경로 접두(드라이브 `X:` 또는 UNC `\\host\share`)의 바이트 길이. 없으면 0.
fn prefix_len(s: &str) -> usize {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return 2;
    }
    if let Some(rest) = s.strip_prefix("\\\\") {
        let mut parts = rest.splitn(3, '\\');
        if let (Some(h), Some(sh)) = (parts.next(), parts.next())
            && !h.is_empty()
            && !sh.is_empty()
        {
            return 2 + h.len() + 1 + sh.len();
        }
    }
    0
}

/// Go `filepath.Clean`(Windows) 의 필요한 만큼: `/`→`\`, 중복·`.` 제거, `..` 해석. 드라이브·UNC 뿌리는 `\` 로 끝난다.
pub(super) fn clean(s: &str) -> String {
    let s = strip_verbatim(s).replace('/', "\\");
    let plen = prefix_len(&s);
    let (prefix, rest) = s.split_at(plen);
    let rooted = rest.starts_with('\\') || prefix.starts_with("\\\\");
    let mut segs: Vec<&str> = Vec::new();
    for seg in rest.split('\\').filter(|x| !x.is_empty() && *x != ".") {
        if seg == ".." {
            if segs.last().is_some_and(|l| *l != "..") {
                segs.pop();
            } else if !rooted {
                segs.push(seg);
            }
        } else {
            segs.push(seg);
        }
    }
    let mut out = String::from(prefix);
    if rooted {
        out.push('\\');
    }
    out.push_str(&segs.join("\\"));
    out
}

/// `to` 값 정리: 끝 구분자를 떼되 뿌리(드라이브·UNC 공유)는 `\` 를 남긴다. 비면 `None`.
/// `E:` 는 "E 드라이브의 현재 폴더" 라 `Join("E:", "a")` 가 `E:a` 가 된다 — 2026-10-23 실기에서 조용히 깨우지 못한 원인.
fn clean_to(s: &str) -> Option<String> {
    let t = strip_verbatim(s.trim());
    let t = t.trim_end_matches(['\\', '/']);
    if t.is_empty() {
        return None;
    }
    let plen = prefix_len(t);
    Some(if plen > 0 && plen == t.len() { clean(&format!("{t}\\")) } else { clean(t) })
}

fn join(to: &str, rest: &str) -> String {
    clean(&format!("{}\\{}", to.trim_end_matches('\\'), rest.replace('/', "\\")))
}

/// 절대 경로(드라이브 또는 UNC 로 시작)인가 — 저장 검증용. 드라이브 상대(`E:a`)·상대 경로는 거부한다.
pub fn is_absolute(path: &str) -> bool {
    let t = &strip_verbatim(path.trim());
    let plen = prefix_len(t);
    plen > 0 && (t.starts_with("\\\\") || t[plen..].starts_with(['\\', '/']))
}

fn lower_clean(s: &str) -> String {
    clean(s).to_lowercase()
}

/// `full` 이 `dir` 아래(또는 `dir` 자신)인가. 대소문자를 가리지 않고 이름이 겹쳐도(`UHD2`) 다른 폴더로 본다.
pub fn within_dir(full: &str, dir: &str) -> bool {
    if full.trim().is_empty() || dir.trim().is_empty() {
        return false;
    }
    let (a, b) = (lower_clean(full), lower_clean(dir));
    a == b || a.starts_with(&format!("{}\\", b.trim_end_matches('\\')))
}

/// 두 경로가 같은 곳인가 (대소문자·끝 구분자 무시)
pub fn same_dir(a: &str, b: &str) -> bool {
    !a.trim().is_empty() && !b.trim().is_empty() && lower_clean(a) == lower_clean(b)
}

/// 규칙 중 하나라도 폴더 루트에 닿는가 — `to` 가 루트 자신이거나 그 위/아래면 닿는다.
/// 닿지 못하면 붙어서 줄을 읽어도 그 폴더는 영영 깨어나지 않는다(조용히 아무 일도 안 일어나는 전형적인 자리) — 저장 때 거부한다.
pub fn map_reaches(rules: &[Mapping], root: &str) -> bool {
    rules
        .iter()
        .filter_map(|r| clean_to(&r.to))
        .any(|to| within_dir(&to, root) || within_dir(root, &to))
}

#[cfg(test)]
mod tests {
    //! 분류: Happy (01·05) / Boundary (02 긴 규칙 우선·03 한 칸 단위·06 드라이브 뿌리) / Validation (04 규칙 없음·07 위험 경로·09 절대 경로) /
    //! Business rule (08 폴더 루트에 닿음). watch_folder `pathmap_test.go`·`config/ddp_test.go` 이식.
    use super::*;

    fn m(from: &str, to: &str) -> Mapping {
        Mapping { from: from.into(), to: to.into() }
    }

    #[test]
    fn t_remote_map_01_root_rule_maps_with_or_without_leading_slash() {
        let mp = Mapper::new(&[m("/", r"F:\UHD")]);
        for (i, want) in [
            ("/a.mxf", r"F:\UHD\a.mxf"),
            ("/1TV/2026/10월/a.mxf", r"F:\UHD\1TV\2026\10월\a.mxf"),
            ("1TV/a.mxf", r"F:\UHD\1TV\a.mxf"),
            ("/뉴스 특보 001.mxf", r"F:\UHD\뉴스 특보 001.mxf"),
        ] {
            assert_eq!(mp.map(i).as_deref(), Some(want), "{i}");
        }
    }

    #[test]
    fn t_remote_map_02_longest_rule_wins() {
        let mp = Mapper::new(&[m("/", r"F:\UHD"), m("/Protools", r"G:\프로툴")]);
        assert_eq!(mp.map("/Protools/mov3.mov").as_deref(), Some(r"G:\프로툴\mov3.mov"));
        assert_eq!(mp.map("/1TV/a.mxf").as_deref(), Some(r"F:\UHD\1TV\a.mxf"));
    }

    #[test]
    fn t_remote_map_03_rules_match_whole_path_segments_only() {
        let mp = Mapper::new(&[m("/Proto", r"G:\짧은")]);
        assert_eq!(mp.map("/Protools/mov3.mov"), None);
        assert_eq!(mp.map("/Proto/x.mov").as_deref(), Some(r"G:\짧은\x.mov"));
        assert_eq!(mp.map("/Proto").as_deref(), Some(r"G:\짧은"));
    }

    #[test]
    fn t_remote_map_04_no_rule_or_empty_target_means_no_mapping() {
        assert_eq!(Mapper::new(&[m("/Protools", r"G:\프로툴")]).map("/1TV/a.mxf"), None);
        assert_eq!(Mapper::new(&[m("/", "")]).map("/a.mxf"), None);
        assert!(Mapper::new(&[]).is_empty());
    }

    #[test]
    fn t_remote_map_05_unc_shares_work_as_targets() {
        let mp = Mapper::new(&[m("/data/incoming", r"\\nas\media")]);
        assert_eq!(mp.map("/data/incoming/a.mov").as_deref(), Some(r"\\nas\media\a.mov"));
        assert_eq!(mp.map("/data/incoming").as_deref(), Some(r"\\nas\media\"));
    }

    #[test]
    fn t_remote_map_06_drive_root_targets_keep_the_separator() {
        for (to, want) in [
            (r"E:\", r"E:\smi-storage\test01.mp4"),
            ("E:", r"E:\smi-storage\test01.mp4"),
            (r"E:\ ", r"E:\smi-storage\test01.mp4"),
            (r"E:\UHD", r"E:\UHD\smi-storage\test01.mp4"),
            (r"E:\UHD\", r"E:\UHD\smi-storage\test01.mp4"),
        ] {
            let mp = Mapper::new(&[m("/", to)]);
            assert_eq!(mp.map("/smi-storage/test01.mp4").as_deref(), Some(want), "to={to:?}");
        }
        let mp = Mapper::new(&[m("/", r"E:\")]);
        assert!(within_dir(&mp.map("/smi-storage/test01.mp4").unwrap(), r"E:\smi-storage"));
        assert_eq!(mp.map("/").as_deref(), Some(r"E:\"));
    }

    #[test]
    fn t_remote_map_07_untrusted_paths_with_dotdot_or_reserved_characters_are_not_mapped() {
        let mp = Mapper::new(&[m("/", r"F:\UHD")]);
        for bad in ["/../Windows/x", "/a/../../b", "/a/b:stream", "/a/b*c", "/a/\0b", "/a/<b>"] {
            assert_eq!(mp.map(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn t_remote_map_08_within_dir_is_case_insensitive_and_segment_exact() {
        for (full, dir) in [
            (r"F:\UHD\a.mxf", r"F:\UHD"),
            (r"F:\UHD\1TV\a.mxf", r"F:\UHD"),
            (r"F:\UHD", r"F:\UHD"),
            (r"f:\uhd\a.mxf", r"F:\UHD"),
            (r"F:\UHD\a.mxf", r"F:\UHD\"),
            (r"F:\UHD\a.mxf", r"F:\"),
            (r"\\nas\media\a.mov", r"\\nas\media"),
        ] {
            assert!(within_dir(full, dir), "{full} in {dir}");
        }
        for (full, dir) in [
            (r"F:\UHD2\a.mxf", r"F:\UHD"),
            (r"E:\UHD\a.mxf", r"F:\UHD"),
            (r"F:\a.mxf", r"F:\UHD"),
            ("", r"F:\UHD"),
            (r"F:\UHD\a.mxf", ""),
            (r"G:\UHD\a.mxf", r"F:\"),
        ] {
            assert!(!within_dir(full, dir), "{full} in {dir}");
        }
    }

    #[test]
    fn t_remote_map_09_map_reaches_the_folder_root_when_a_target_is_the_root_above_or_below_it() {
        for to in [r"F:\UHD", r"F:\", r"f:\uhd", r"F:\UHD\1TV"] {
            assert!(map_reaches(&[m("/", to)], r"F:\UHD"), "to={to}");
        }
        for to in [r"G:\UHD", r"F:\UHD2", ""] {
            assert!(!map_reaches(&[m("/", to)], r"F:\UHD"), "to={to}");
        }
        assert!(!map_reaches(&[], r"F:\UHD"));
        // 규칙 중 하나만 닿아도 된다
        assert!(map_reaches(&[m("/a", r"G:\x"), m("/b", r"F:\UHD")], r"F:\UHD"));
    }

    #[test]
    fn t_remote_map_10_absolute_target_paths_only() {
        for ok in [r"F:\UHD", r"F:\", "F:/UHD", r"\\nas\media", r"\\nas\media\sub"] {
            assert!(is_absolute(ok), "{ok}");
        }
        for bad in ["", "UHD", r"\UHD", "F:UHD", r"\\nas", r"\\nas\", "/data"] {
            assert!(!is_absolute(bad), "{bad}");
        }
    }
}
