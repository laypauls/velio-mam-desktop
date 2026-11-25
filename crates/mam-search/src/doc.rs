//! 색인할 자산 문서. 값은 모두 SQLite 가 원천이며, 인덱스는 파생 데이터다.

use tantivy::TantivyDocument;

use crate::error::{Result, SearchError};
use crate::preprocess::split_extension;
use crate::schema::Fields;
use crate::userfield::{UserFieldValue, add_user_fields};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    Image,
    Video,
    Audio,
}

impl MediaType {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaType::Image => "image",
            MediaType::Video => "video",
            MediaType::Audio => "audio",
        }
    }
}

/// `file_locations.presence` 와 같은 값. 오프라인 여부는 색인하지 않는다 (`volume_id` 필터로 처리).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presence {
    Present,
    Missing,
}

impl Presence {
    pub fn as_str(self) -> &'static str {
        match self {
            Presence::Present => "present",
            Presence::Missing => "missing",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AssetDoc {
    pub asset_id: u64,
    /// `media_assets.revision` (>= 1). 낮은 revision 의 upsert 는 무시된다.
    pub revision: u64,
    /// 확장자를 포함한 전체 파일명. 확장자는 색인 시 분리한다.
    pub file_name: String,
    pub user_tags: Vec<String>,
    pub ai_keywords: Vec<String>,
    pub ai_description: String,
    pub user_memo: String,
    /// 사용자 지정 제목 (P-title). 없으면 빈 문자열 — 파일명과 별개로 검색되고, 제목순 정렬 키가 된다.
    pub title: String,
    pub transcript: String,
    pub media_type: MediaType,
    pub volume_id: u64,
    pub presence: Presence,
    /// 유닉스 시각(초). 없으면 필드를 쓰지 않는다.
    pub captured_at: Option<i64>,
    pub file_size: u64,
    pub duration_ms: Option<u64>,
    /// 사용자 즐겨찾기 (P2-7)
    pub favorite: bool,
    /// 별점 0..=5 (0 = 미평가)
    pub rating: u8,
    /// 소속 컬렉션 id (P2-7)
    pub collection_ids: Vec<u64>,
    /// 태그 id — 태그 필터용 (이름은 `user_tags`)
    pub tag_ids: Vec<u64>,
    /// `media_assets.modified_at` (UTC ms) — 수정일 정렬용 (P2-5)
    pub modified_at: i64,
    /// 소속 관리 폴더 (없으면 폴더 범위 필터에 걸리지 않는다) (P2-5)
    pub folder_id: Option<u64>,
    /// 관리 폴더 루트 기준 상위 디렉터리의 `path_key` (루트 직속 = `""`) (P2-5)
    pub dir_key: String,
    /// 사용자 정의 메타데이터 필드 값 (삭제 중인 필드·선택지의 값은 제외하고 채운다). 비어 있으면 사용자 필드 색인 없음.
    pub user_fields: Vec<UserFieldValue>,
}

/// 촬영일 대체 키(유닉스 초): 촬영일(초)이 있으면 그 값, 없으면 수정일(UTC ms)을 초로 내림한 값.
/// 원천(`media_metadata.captured_at`)에는 쓰지 않고 색인에만 합성한다.
pub fn captured_or_modified(captured_at_secs: Option<i64>, modified_at_ms: i64) -> i64 {
    captured_at_secs.unwrap_or_else(|| modified_at_ms.div_euclid(1000))
}

/// 폴더 범위 색인 토큰: `"<folder_id>:<dir_key>"`. 재귀 범위는 조상 토큰, 비재귀는 정확 토큰으로 맞춘다.
pub fn dir_token(folder_id: u64, dir_key: &str) -> String {
    format!("{folder_id}:{dir_key}")
}

/// 이름 정렬 키: 소문자 NFC 파일명 UTF-8 앞 16바이트를 (hi, lo) 로. 16바이트가 같으면 `asset_id` 순.
pub fn name_sort_key(file_name: &str) -> (u64, u64) {
    use unicode_normalization::UnicodeNormalization;
    let lowered: String = file_name.nfc().collect::<String>().to_lowercase();
    let mut buf = [0u8; 16];
    let bytes = lowered.as_bytes();
    let n = bytes.len().min(16);
    buf[..n].copy_from_slice(&bytes[..n]);
    let (hi, lo) = buf.split_at(8);
    (
        u64::from_be_bytes(hi.try_into().unwrap_or([0; 8])),
        u64::from_be_bytes(lo.try_into().unwrap_or([0; 8])),
    )
}

/// 파일명 부분 일치 전략에 필요한 보조 필드를 만들지 여부.
#[derive(Clone, Copy, Debug)]
pub struct BuildFlags {
    pub ngram: bool,
    pub raw: bool,
}

impl AssetDoc {
    pub(crate) fn to_tantivy(&self, f: &Fields, flags: BuildFlags) -> Result<TantivyDocument> {
        if self.revision == 0 {
            return Err(SearchError::InvalidInput("revision must be >= 1".into()));
        }
        let mut d = TantivyDocument::default();
        d.add_u64(f.asset_id, self.asset_id);
        d.add_u64(f.revision, self.revision);
        let (stem, ext) = split_extension(&self.file_name);
        if !stem.is_empty() {
            d.add_text(f.file_name, &stem);
            if flags.ngram {
                d.add_text(f.file_name_ngram, &stem);
            }
            if flags.raw {
                d.add_text(f.file_name_raw, &stem);
            }
        }
        if let Some(ext) = ext {
            d.add_text(f.ext, ext);
        }
        for (field, values) in [
            (f.user_tags, &self.user_tags),
            (f.ai_keywords, &self.ai_keywords),
        ] {
            for v in values.iter().filter(|v| !v.trim().is_empty()) {
                d.add_text(field, v);
            }
        }
        for (field, v) in [
            (f.ai_description, &self.ai_description),
            (f.user_memo, &self.user_memo),
            (f.title, &self.title),
            (f.transcript, &self.transcript),
        ] {
            if !v.trim().is_empty() {
                d.add_text(field, v);
            }
        }
        d.add_text(f.media_type, self.media_type.as_str());
        d.add_u64(f.volume_id, self.volume_id);
        d.add_text(f.presence, self.presence.as_str());
        if let Some(v) = self.captured_at {
            d.add_i64(f.captured_at, v);
        }
        d.add_i64(
            f.captured_or_modified,
            captured_or_modified(self.captured_at, self.modified_at),
        );
        d.add_u64(f.file_size, self.file_size);
        if let Some(v) = self.duration_ms {
            d.add_u64(f.duration_ms, v);
        }
        d.add_u64(f.favorite, u64::from(self.favorite));
        d.add_u64(f.rating, u64::from(self.rating.min(5)));
        for id in &self.collection_ids {
            d.add_u64(f.collection_id, *id);
        }
        for id in &self.tag_ids {
            d.add_u64(f.tag_id, *id);
        }
        d.add_i64(f.modified_at, self.modified_at);
        let (hi, lo) = name_sort_key(&self.file_name);
        d.add_u64(f.name_hi, hi);
        d.add_u64(f.name_lo, lo);
        let title = self.title.trim();
        let (hi, lo) = name_sort_key(if title.is_empty() { &self.file_name } else { title });
        d.add_u64(f.title_hi, hi);
        d.add_u64(f.title_lo, lo);
        if let Some(fid) = self.folder_id {
            d.add_text(f.dir_exact, dir_token(fid, &self.dir_key));
            // 조상 토큰: 루트(`<fid>:`) + 각 상위 디렉터리 접두
            d.add_text(f.dir_anc, dir_token(fid, ""));
            if !self.dir_key.is_empty() {
                let mut end = 0;
                for seg in self.dir_key.split('/') {
                    end += seg.len();
                    d.add_text(f.dir_anc, dir_token(fid, &self.dir_key[..end]));
                    end += 1;
                }
            }
        }
        add_user_fields(&mut d, f, &self.user_fields);
        Ok(d)
    }
}

#[cfg(test)]
mod tests {
    // Categories: Happy (사용자 데이터 필드가 문서에 실린다) / Boundary (빈 값은 필드를 쓰지 않는다)
    use tantivy::schema::Value;

    use super::*;
    use crate::schema::build_schema;

    fn sample() -> AssetDoc {
        AssetDoc {
            asset_id: 7,
            revision: 3,
            file_name: "a.jpg".into(),
            user_tags: vec![],
            ai_keywords: vec![],
            ai_description: String::new(),
            user_memo: String::new(),
            title: String::new(),
            transcript: String::new(),
            media_type: MediaType::Image,
            volume_id: 1,
            presence: Presence::Present,
            captured_at: None,
            file_size: 1,
            duration_ms: None,
            favorite: false,
            rating: 0,
            collection_ids: vec![],
            tag_ids: vec![],
            modified_at: 0,
            folder_id: None,
            dir_key: String::new(),
            user_fields: vec![],
        }
    }

    fn u64s(d: &TantivyDocument, f: tantivy::schema::Field) -> Vec<u64> {
        d.get_all(f).filter_map(|v| v.as_u64()).collect()
    }

    #[test]
    fn t_sync_15_user_data_fields_are_written_to_the_tantivy_document() {
        // Arrange
        let (_, f) = build_schema();
        let flags = BuildFlags {
            ngram: false,
            raw: false,
        };
        let mut doc = sample();
        doc.favorite = true;
        doc.collection_ids = vec![5, 9];
        doc.tag_ids = vec![2];

        // Act
        let d = doc.to_tantivy(&f, flags).unwrap();
        let empty = sample().to_tantivy(&f, flags).unwrap();

        // Assert
        assert_eq!(u64s(&d, f.favorite), [1]);
        assert_eq!(u64s(&d, f.collection_id), [5, 9]);
        assert_eq!(u64s(&d, f.tag_id), [2]);
        assert_eq!(u64s(&empty, f.favorite), [0]);
        assert!(u64s(&empty, f.collection_id).is_empty());
        assert!(u64s(&empty, f.tag_id).is_empty());
    }

    fn i64s(d: &TantivyDocument, f: tantivy::schema::Field) -> Vec<i64> {
        d.get_all(f).filter_map(|v| v.as_i64()).collect()
    }

    #[test]
    fn t_capfb_doc_01_captured_or_modified_prefers_capture_and_floors_modified_ms_to_seconds() {
        // Arrange
        let (_, f) = build_schema();
        let flags = BuildFlags { ngram: false, raw: false };
        let mut with_capture = sample();
        with_capture.captured_at = Some(1_700_000_000);
        with_capture.modified_at = 5_000_000_000_000;
        let mut without = sample();
        without.modified_at = 1_642_248_000_999;

        // Act
        let a = with_capture.to_tantivy(&f, flags).unwrap();
        let b = without.to_tantivy(&f, flags).unwrap();

        // Assert: 원래 captured_at 은 그대로(촬영일 없으면 비어 있음), 합성 필드만 대체값을 가진다
        assert_eq!(i64s(&a, f.captured_at), [1_700_000_000]);
        assert_eq!(i64s(&a, f.captured_or_modified), [1_700_000_000]);
        assert!(i64s(&b, f.captured_at).is_empty());
        assert_eq!(i64s(&b, f.captured_or_modified), [1_642_248_000]);
    }

    #[test]
    fn t_capfb_doc_02_modified_ms_boundaries() {
        // Boundary: 0 / 999 / 1000 / 음수(1970 이전은 내림)
        assert_eq!(captured_or_modified(None, 0), 0);
        assert_eq!(captured_or_modified(None, 999), 0);
        assert_eq!(captured_or_modified(None, 1000), 1);
        assert_eq!(captured_or_modified(None, -1), -1);
        assert_eq!(captured_or_modified(Some(0), 9_999), 0, "capture 0 is a value, not 'missing'");
    }

    fn texts(d: &TantivyDocument, f: tantivy::schema::Field) -> Vec<String> {
        d.get_all(f)
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    #[test]
    fn t_coord_15_folder_tokens_cover_every_ancestor_and_the_exact_directory() {
        // Arrange
        let (_, f) = build_schema();
        let flags = BuildFlags {
            ngram: false,
            raw: false,
        };
        let mut nested = sample();
        nested.folder_id = Some(4);
        nested.dir_key = "a/한글/c".into();
        let mut at_root = sample();
        at_root.folder_id = Some(4);

        // Act
        let n = nested.to_tantivy(&f, flags).unwrap();
        let r = at_root.to_tantivy(&f, flags).unwrap();
        let none = sample().to_tantivy(&f, flags).unwrap();

        // Assert
        assert_eq!(texts(&n, f.dir_exact), ["4:a/한글/c"]);
        assert_eq!(
            texts(&n, f.dir_anc),
            ["4:", "4:a", "4:a/한글", "4:a/한글/c"]
        );
        assert_eq!(texts(&r, f.dir_exact), ["4:"]);
        assert_eq!(texts(&r, f.dir_anc), ["4:"]);
        assert!(
            texts(&none, f.dir_anc).is_empty(),
            "no folder -> no scope tokens"
        );
    }

    #[test]
    fn t_coord_16_name_sort_key_is_case_insensitive_and_prefix_ordered() {
        assert_eq!(name_sort_key("Apple.JPG"), name_sort_key("apple.jpg"));
        assert!(name_sort_key("apple") < name_sort_key("banana"));
        assert!(name_sort_key("banana") < name_sort_key("가나"));
        assert_eq!(name_sort_key(""), (0, 0));
        // 16바이트 이후만 다르면 같은 키 (asset_id 로 순서를 정한다)
        assert_eq!(
            name_sort_key("0123456789abcdefX"),
            name_sort_key("0123456789abcdefY")
        );
    }
}
