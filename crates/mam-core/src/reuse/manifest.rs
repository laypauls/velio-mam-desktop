//! 내보내기 매니페스트 `mam-export.csv` — 파일명·원본 경로·태그·메모·촬영일.
//!
//! UTF-8 BOM + CRLF (Excel 한글 호환). 태그·메모는 사용자 입력이라 `=` `+` `-` `@` 로 시작하면 수식으로 실행되지
//! 않도록 앞에 `'` 를 붙인다 (CSV 인젝션 방어).

/// 매니페스트 파일 이름
pub const MANIFEST_NAME: &str = "mam-export.csv";

pub struct Row<'a> {
    pub file_name: &'a str,
    pub original_path: &'a str,
    pub tags: &'a [String],
    pub memo: &'a str,
    /// UTC Unix ms
    pub captured_at: Option<i64>,
}

fn cell(out: &mut String, raw: &str) {
    let guarded = raw.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let needs_quote = guarded || raw.contains([',', '"', '\r', '\n']);
    if !needs_quote {
        out.push_str(raw);
        return;
    }
    out.push('"');
    if guarded {
        out.push('\'');
    }
    out.push_str(&raw.replace('"', "\"\""));
    out.push('"');
}

pub fn render(rows: &[Row<'_>]) -> Vec<u8> {
    let mut out = String::from("\u{feff}file_name,original_path,tags,memo,captured_at\r\n");
    for r in rows {
        cell(&mut out, r.file_name);
        out.push(',');
        cell(&mut out, r.original_path);
        out.push(',');
        cell(&mut out, &r.tags.join("; "));
        out.push(',');
        cell(&mut out, r.memo);
        out.push(',');
        if let Some(ms) = r.captured_at {
            out.push_str(&iso_utc(ms));
        }
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// `YYYY-MM-DDTHH:MM:SSZ` (UTC)
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// 1970-01-01 기준 일 수 -> (년, 월, 일) (Howard Hinnant 의 civil_from_days)
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    // Categories: Happy / Boundary (epoch, 윤년) / Validation (CSV 인젝션·인용)
    use super::*;

    #[test]
    fn t_reuse_12_iso_utc_handles_epoch_and_leap_day() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_709_210_096_000), "2024-02-29T12:34:56Z");
        assert_eq!(iso_utc(-1000), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn t_reuse_13_csv_quotes_special_cells_and_neutralizes_formulas() {
        // Arrange
        let tags = vec!["a".to_string(), "=cmd".to_string()];
        let rows = [Row {
            file_name: "x,y.jpg",
            original_path: r"D:\a\x,y.jpg",
            tags: &tags,
            memo: "=1+1",
            captured_at: Some(0),
        }];

        // Act
        let text = String::from_utf8(render(&rows)).unwrap();

        // Assert
        assert!(text.starts_with('\u{feff}'));
        let line = text.lines().nth(1).unwrap();
        assert_eq!(
            line,
            "\"x,y.jpg\",\"D:\\a\\x,y.jpg\",a; =cmd,\"'=1+1\",1970-01-01T00:00:00Z"
        );
    }
}
