#![allow(dead_code)]

use mam_search::{
    AssetDoc, FilenameStrategy, IndexOptions, MediaType, Presence, SearchIndex, SearchRequest,
};
use tempfile::TempDir;

/// 기본값이 채워진 문서. 필요한 필드만 덮어써서 쓴다.
pub fn doc(asset_id: u64, file_name: &str) -> AssetDoc {
    AssetDoc {
        asset_id,
        revision: 1,
        file_name: file_name.to_string(),
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
        file_size: 1000,
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

/// 테스트에서는 writer 자원을 줄인다.
pub fn small(mut o: IndexOptions) -> IndexOptions {
    o.writer_heap_bytes = 15_000_000;
    o.writer_threads = 1;
    o
}

/// B·C 보조 필드를 모두 만드는 인덱스.
pub fn open_all(dir: &TempDir) -> SearchIndex {
    SearchIndex::open_or_create(&dir.path().join("idx"), small(IndexOptions::all())).unwrap()
}

pub fn req(text: &str, strategy: FilenameStrategy) -> SearchRequest {
    SearchRequest {
        text: text.to_string(),
        strategy,
        ..Default::default()
    }
}

/// 정렬된 asset_id 목록 (순서 무관 비교용).
pub fn ids(index: &SearchIndex, r: &SearchRequest) -> Vec<u64> {
    let mut v: Vec<u64> = index
        .search(r)
        .unwrap()
        .iter()
        .map(|h| h.asset_id)
        .collect();
    v.sort_unstable();
    v
}

/// 문서들을 넣고 commit 한 인덱스.
pub fn indexed(dir: &TempDir, docs: &[AssetDoc]) -> SearchIndex {
    let idx = open_all(dir);
    for d in docs {
        idx.upsert(d).unwrap();
    }
    idx.commit().unwrap();
    idx
}
