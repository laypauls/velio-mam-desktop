//! SQLite -> `AssetDoc`. 색인 재구축과 품질 평가가 같은 질의를 쓴다 (한 번의 스트리밍 조인).

use mam_search::{AssetDoc, MediaType, Presence};
use rusqlite::{Connection, Row, params};

/// 자산 + 활성 위치 + 메타데이터 + 태그(상관 부질의, 자산당 1행) + 측정용 AI 텍스트(D5).
/// 태그는 `char(31)` 로 이어 붙인다.
macro_rules! doc_select {
    () => {
        "SELECT a.asset_id, a.revision, a.media_type, a.file_size,
        l.volume_id, l.rel_path, l.presence,
        m.captured_at, m.duration_ms,
        (SELECT group_concat(t.name, char(31)) FROM asset_tags at
           JOIN tags t ON t.tag_id = at.tag_id WHERE at.asset_id = a.asset_id),
        x.ai_description, x.ai_keywords, x.user_memo
 FROM media_assets a
 JOIN file_locations l ON l.asset_id = a.asset_id AND l.detached_at IS NULL
 LEFT JOIN media_metadata m ON m.asset_id = a.asset_id
 LEFT JOIN poc_asset_text x ON x.asset_id = a.asset_id
 WHERE a.status = 'active'"
    };
}

/// 전체 스트리밍 질의 (자산 id 순).
pub const ALL_DOCS_SQL: &str = concat!(doc_select!(), " ORDER BY a.asset_id");

pub fn doc_from_row(r: &Row) -> rusqlite::Result<AssetDoc> {
    let rel_path: String = r.get(5)?;
    let media_type = match r.get::<_, String>(2)?.as_str() {
        "video" => MediaType::Video,
        "audio" => MediaType::Audio,
        _ => MediaType::Image,
    };
    let presence = if r.get::<_, String>(6)? == "missing" {
        Presence::Missing
    } else {
        Presence::Present
    };
    let tags: Option<String> = r.get(9)?;
    let keywords: Option<String> = r.get(11)?;
    Ok(AssetDoc {
        asset_id: r.get::<_, i64>(0)? as u64,
        revision: r.get::<_, i64>(1)? as u64,
        file_name: rel_path.rsplit('/').next().unwrap_or("").to_string(),
        user_tags: tags
            .map(|t| t.split('\u{1f}').map(str::to_string).collect())
            .unwrap_or_default(),
        ai_keywords: keywords
            .map(|k| k.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default(),
        ai_description: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
        user_memo: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
        title: String::new(),
        transcript: String::new(),
        media_type,
        volume_id: r.get::<_, i64>(4)? as u64,
        presence,
        // 인덱스의 captured_at 은 유닉스 초 (SQLite 는 ms)
        captured_at: r.get::<_, Option<i64>>(7)?.map(|ms| ms / 1000),
        file_size: r.get::<_, i64>(3)? as u64,
        duration_ms: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        favorite: false,
        rating: 0,
        collection_ids: vec![],
        tag_ids: vec![],
        modified_at: 0,
        folder_id: None,
        dir_key: String::new(),
        user_fields: vec![],
    })
}

/// 자산 1건 (단건 갱신 측정용).
pub fn load_one(conn: &Connection, asset_id: u64) -> rusqlite::Result<AssetDoc> {
    let sql = concat!(doc_select!(), " AND a.asset_id = ?1");
    conn.query_row(sql, params![asset_id as i64], doc_from_row)
}
