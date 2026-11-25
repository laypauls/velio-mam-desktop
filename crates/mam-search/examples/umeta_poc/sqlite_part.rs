//! D1 스키마를 임시 SQLite 파일에 직접 만들어 하이드레이션·대량 지정·DB 크기를 측정한다 (마이그레이션 파일 아님).

use std::path::Path;
use std::time::{Duration, Instant};

use rusqlite::{Connection, params, params_from_iter};

use crate::data::{Ty, Val, gen_values, option_id, row_count, specs};
use crate::rng::Rng;
use crate::stats::summarize;

const DDL: &str = "
CREATE TABLE media_assets (
  asset_id INTEGER PRIMARY KEY AUTOINCREMENT, media_type TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'active', revision INTEGER NOT NULL DEFAULT 1,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL) STRICT;
CREATE TABLE search_outbox (
  event_id INTEGER PRIMARY KEY AUTOINCREMENT, asset_id INTEGER NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  event_type TEXT NOT NULL CHECK (event_type IN ('upsert','delete')),
  priority INTEGER NOT NULL DEFAULT 1 CHECK (priority >= 0),
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','done','failed')),
  attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT, created_at INTEGER NOT NULL) STRICT;
CREATE INDEX idx_search_outbox_pending ON search_outbox (status, priority, event_id);
CREATE TABLE metadata_fields (
  field_id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL, name_key TEXT NOT NULL UNIQUE,
  value_type TEXT NOT NULL CHECK (value_type IN ('keyword','number','date','boolean','enum','multi_enum')),
  position INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL) STRICT;
CREATE TABLE metadata_field_options (
  option_id INTEGER PRIMARY KEY AUTOINCREMENT,
  field_id INTEGER NOT NULL REFERENCES metadata_fields(field_id) ON DELETE CASCADE,
  label TEXT NOT NULL, label_key TEXT NOT NULL, position INTEGER NOT NULL,
  UNIQUE (field_id, label_key)) STRICT;
CREATE TABLE asset_metadata_values (
  asset_id INTEGER NOT NULL REFERENCES media_assets(asset_id) ON DELETE CASCADE,
  field_id INTEGER NOT NULL REFERENCES metadata_fields(field_id) ON DELETE CASCADE,
  ord INTEGER NOT NULL DEFAULT 0,
  value_text TEXT, value_int INTEGER, value_real REAL, updated_at INTEGER NOT NULL,
  PRIMARY KEY (asset_id, field_id, ord),
  CHECK ((value_text IS NOT NULL) + (value_int IS NOT NULL) + (value_real IS NOT NULL) = 1)
) STRICT, WITHOUT ROWID;
";
const IDX: &str = "CREATE INDEX idx_amv_field_asset ON asset_metadata_values (field_id, asset_id);";

const UPSERT: &str = "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_text, value_int, value_real, updated_at)
 VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6)
 ON CONFLICT (asset_id, field_id, ord) DO UPDATE SET value_text = excluded.value_text,
   value_int = excluded.value_int, value_real = excluded.value_real, updated_at = excluded.updated_at";

fn conn(path: &Path) -> Connection {
    let c = Connection::open(path).unwrap();
    c.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;").unwrap();
    c
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn file_mb(p: &Path) -> f64 {
    let mut t = 0u64;
    for suffix in ["", "-wal", "-shm"] {
        let mut s = p.as_os_str().to_os_string();
        s.push(suffix);
        t += std::fs::metadata(&s).map(|m| m.len()).unwrap_or(0);
    }
    t as f64 / 1_048_576.0
}

fn plan(c: &Connection, sql: &str) {
    let mut st = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    let rows = st
        .query_map(params_from_iter(std::iter::repeat_n(1i64, sql.matches('?').count())), |r| r.get::<_, String>(3))
        .unwrap();
    for r in rows {
        println!("    plan: {}", r.unwrap());
    }
}

pub fn load(path: &Path, n: u64) {
    let n = n as i64;
    for s in ["", "-wal", "-shm"] {
        let mut p = path.as_os_str().to_os_string();
        p.push(s);
        let _ = std::fs::remove_file(p);
    }
    let c = conn(path);
    c.execute_batch(DDL).unwrap();
    let sp = specs();
    let t0 = Instant::now();
    {
        let tx = c.unchecked_transaction().unwrap();
        for s in &sp {
            tx.execute(
                "INSERT INTO metadata_fields (field_id,name,name_key,value_type,position,created_at,updated_at) VALUES (?1,?2,?2,?3,?1,0,0)",
                params![s.id, format!("field{}", s.id), s.ty.sql()],
            )
            .unwrap();
            if matches!(s.ty, Ty::Enum | Ty::Multi) {
                for k in 0..s.card {
                    tx.execute(
                        "INSERT INTO metadata_field_options (option_id,field_id,label,label_key,position) VALUES (?1,?2,?3,?3,?4)",
                        params![option_id(s.id, k), s.id, format!("opt{k}"), k as i64],
                    )
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
    }
    let mut rows_total = 0u64;
    let mut start = 1i64;
    while start <= n {
        let end = (start + 49_999).min(n);
        let tx = c.unchecked_transaction().unwrap();
        {
            let mut a = tx.prepare_cached("INSERT INTO media_assets (asset_id, media_type, revision, created_at, updated_at) VALUES (?1,?2,1,0,0)").unwrap();
            let mut v = tx.prepare_cached("INSERT INTO asset_metadata_values (asset_id,field_id,ord,value_text,value_int,value_real,updated_at) VALUES (?1,?2,?3,?4,?5,?6,0)").unwrap();
            for id in start..=end {
                a.execute(params![id, crate::data::media_type(id as u64)]).unwrap();
                let vals = gen_values(id as u64, &sp, 0);
                rows_total += row_count(&vals) as u64;
                for (fid, val) in &vals {
                    match val {
                        Val::Kw(s) => v.execute(params![id, fid, 0, s, None::<i64>, None::<f64>]),
                        Val::Num(x) => v.execute(params![id, fid, 0, None::<String>, None::<i64>, x]),
                        Val::Date(x) | Val::Bool(x) | Val::Enum(x) => v.execute(params![id, fid, 0, None::<String>, x, None::<f64>]),
                        Val::Multi(xs) => {
                            for x in xs {
                                v.execute(params![id, fid, x, None::<String>, x, None::<f64>]).unwrap();
                            }
                            Ok(0)
                        }
                    }
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
        start = end + 1;
        if end % 250_000 == 0 {
            println!("  loaded {end} assets, {rows_total} value rows, {:.0}s", secs(t0.elapsed()));
        }
    }
    let load_t = t0.elapsed();
    let ti = Instant::now();
    c.execute_batch(IDX).unwrap();
    let idx_t = ti.elapsed();
    let tc = Instant::now();
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); ANALYZE;").unwrap();
    println!(
        "LOAD assets={n} value_rows={rows_total}: insert {:.1}s ({:.0} rows/s), CREATE INDEX(field_id,asset_id) {:.1}s, checkpoint+ANALYZE {:.1}s, db_file={:.0} MB",
        secs(load_t), rows_total as f64 / secs(load_t), secs(idx_t), secs(tc.elapsed()), file_mb(path)
    );
    // 테이블/인덱스별 크기
    match c.prepare("SELECT name, SUM(pgsize) FROM dbstat GROUP BY name ORDER BY 2 DESC LIMIT 8") {
        Ok(mut st) => {
            for r in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))).unwrap() {
                let (n, s) = r.unwrap();
                println!("    {n}: {:.0} MB", s as f64 / 1_048_576.0);
            }
        }
        Err(e) => println!("    dbstat unavailable: {e}"),
    }
}

fn pick_ids(r: &mut Rng, n: u64, k: usize) -> Vec<i64> {
    let mut v = std::collections::BTreeSet::new();
    while v.len() < k {
        v.insert(r.range(1, n) as i64);
    }
    v.into_iter().collect()
}

pub fn measure(path: &Path, n: u64) {
    let c = conn(path);
    let mut r = Rng::new(5);
    println!("\n[하이드레이션]");
    let q1 = "SELECT field_id, ord, value_text, value_int, value_real FROM asset_metadata_values WHERE asset_id = ?1";
    plan(&c, q1);
    let mut st = c.prepare_cached(q1).unwrap();
    let mut run1 = |id: i64| {
        let t = Instant::now();
        let mut rows = st.query([id]).unwrap();
        let mut k = 0;
        while let Some(row) = rows.next().unwrap() {
            let _: i64 = row.get(0).unwrap();
            k += 1;
        }
        (t.elapsed(), k)
    };
    for _ in 0..200 {
        run1(r.range(1, n) as i64);
    }
    let (mut ts, mut kk) = (Vec::new(), 0);
    for _ in 0..3000 {
        let (d, k) = run1(r.range(1, n) as i64);
        ts.push(d);
        kk += k;
    }
    let s = summarize(&ts);
    println!("  단건 asset_id=? (평균 {:.1}행): P50 {:.3} / P95 {:.3} / P99 {:.3} ms (n=3000)", kk as f64 / 3000.0, s.p50_ms, s.p95_ms, s.p99_ms);
    drop(st);

    let in50 = format!("SELECT asset_id, field_id, ord, value_text, value_int, value_real FROM asset_metadata_values WHERE asset_id IN ({})", vec!["?"; 50].join(","));
    plan(&c, &in50);
    let mut ts = Vec::new();
    let mut ts_n1 = Vec::new();
    for i in 0..1000 {
        let ids = pick_ids(&mut r, n, 50);
        let t = Instant::now();
        let mut st = c.prepare_cached(&in50).unwrap();
        let mut rows = st.query(params_from_iter(ids.iter())).unwrap();
        let mut k = 0;
        while let Some(_row) = rows.next().unwrap() {
            k += 1;
        }
        ts.push(t.elapsed());
        let _ = k;
        if i < 200 {
            let t = Instant::now();
            let mut st = c.prepare_cached(q1).unwrap();
            for id in &ids {
                let mut rows = st.query([id]).unwrap();
                while let Some(_row) = rows.next().unwrap() {}
            }
            ts_n1.push(t.elapsed());
        }
    }
    let s = summarize(&ts);
    println!("  50건 IN (한 쿼리): P50 {:.3} / P95 {:.3} / P99 {:.3} ms (n=1000)", s.p50_ms, s.p95_ms, s.p99_ms);
    let s = summarize(&ts_n1);
    println!("  50건을 단건 50회 (참고, prepared): P50 {:.3} / P95 {:.3} / P99 {:.3} ms (n=200)", s.p50_ms, s.p95_ms, s.p99_ms);

    // 워커: 10,000 자산을 하나씩 읽기 (아웃박스 처리 중 값 조회 비용)
    let ids = pick_ids(&mut r, n, 10_000);
    let t = Instant::now();
    let mut st = c.prepare_cached(q1).unwrap();
    let mut cnt = 0;
    for id in &ids {
        let mut rows = st.query([id]).unwrap();
        while let Some(_row) = rows.next().unwrap() {
            cnt += 1;
        }
    }
    drop(st);
    println!("  워커식 10,000 자산 값 조회 (단건 반복): {:.0} ms ({cnt}행)", ms(t.elapsed()));

    // 전체 재구축용 스트리밍 스캔
    let t = Instant::now();
    let mut st = c.prepare("SELECT asset_id, field_id, ord, value_text, value_int, value_real FROM asset_metadata_values ORDER BY asset_id").unwrap();
    plan(&c, "SELECT asset_id, field_id, ord, value_text, value_int, value_real FROM asset_metadata_values ORDER BY asset_id");
    let mut rows = st.query([]).unwrap();
    let mut cnt = 0u64;
    while let Some(_row) = rows.next().unwrap() {
        cnt += 1;
    }
    drop(rows);
    drop(st);
    println!("  전체 스캔(재구축용, asset_id 순): {cnt}행 {:.1} s", secs(t.elapsed()));

    // 필드 단위 대상 조회 (재색인·삭제용)
    for fid in [3, 16] {
        let q = "SELECT asset_id FROM asset_metadata_values WHERE field_id = ?1";
        if fid == 3 {
            plan(&c, q);
        }
        let t = Instant::now();
        let mut st = c.prepare(q).unwrap();
        let v: Vec<i64> = st.query_map([fid], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        println!("  field_id={fid} 대상 자산 조회: {} 건 {:.0} ms", v.len(), ms(t.elapsed()));
    }
    let t = Instant::now();
    let cnt: i64 = c.query_row("SELECT count(*) FROM asset_metadata_values WHERE field_id = 16 AND value_int = 1601", [], |r| r.get(0)).unwrap();
    println!("  (참고) value_int 로 선택지 사용 건수: {cnt} 건 {:.0} ms — field_id 인덱스는 value 열이 없어 필드 행 전체를 읽음", ms(t.elapsed()));

    bulk(&c, n, &mut r);
    field_delete(&c, path);
}

/// 대량 지정: 필드 3개(keyword f1, date f10, enum f16) × `count` 자산, revision 증가 + outbox 기록, 한 트랜잭션.
fn bulk(c: &Connection, n: u64, r: &mut Rng) {
    println!("\n[대량 지정 트랜잭션: 3 필드 × N 자산, revision++ + outbox insert, 한 트랜잭션, WAL/synchronous=NORMAL]");
    println!("| 방식 | 자산 수 | 실행 | ms (실행별) | 중앙값 ms |");
    println!("|---|---|---|---|---|");
    for (count, runs) in [(1_000usize, 5), (10_000, 5)] {
        // A: 자산별 prepared 문 반복
        let mut tms = Vec::new();
        for run in 0..runs {
            let ids = pick_ids(r, n, count);
            let now = 1_700_000_000_000i64 + run as i64;
            let t = Instant::now();
            let tx = c.unchecked_transaction().unwrap();
            {
                let mut up = tx.prepare_cached(UPSERT).unwrap();
                let mut bump = tx.prepare_cached("UPDATE media_assets SET revision = revision + 1, updated_at = ?2 WHERE asset_id = ?1 RETURNING revision").unwrap();
                let mut ob = tx.prepare_cached("INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at) VALUES (?1,?2,'upsert',?3,?4)").unwrap();
                for (i, id) in ids.iter().enumerate() {
                    up.execute(params![id, 1, format!("값{}", i % 50), None::<i64>, None::<f64>, now]).unwrap();
                    up.execute(params![id, 10, None::<String>, 20261120i64, None::<f64>, now]).unwrap();
                    up.execute(params![id, 16, None::<String>, option_id(16, i % 4), None::<f64>, now]).unwrap();
                    let rev: i64 = bump.query_row(params![id, now], |r| r.get(0)).unwrap();
                    ob.execute(params![id, rev, if i < 1000 { 0 } else { 1 }, now]).unwrap();
                }
            }
            tx.commit().unwrap();
            tms.push(ms(t.elapsed()));
        }
        row("A 자산별 prepared 반복", count, &tms);

        // B: 임시 테이블 + 집합 SQL
        let mut tms = Vec::new();
        for run in 0..runs {
            let ids = pick_ids(r, n, count);
            let now = 1_700_000_100_000i64 + run as i64;
            let t = Instant::now();
            let tx = c.unchecked_transaction().unwrap();
            tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS tmp_ids (seq INTEGER PRIMARY KEY, asset_id INTEGER NOT NULL); DELETE FROM tmp_ids;").unwrap();
            {
                let mut ins = tx.prepare_cached("INSERT INTO tmp_ids (seq, asset_id) VALUES (?1, ?2)").unwrap();
                for (i, id) in ids.iter().enumerate() {
                    ins.execute(params![i as i64, id]).unwrap();
                }
            }
            let up = |fid: i64, t: &str, i: &str| {
                format!(
                    "INSERT INTO asset_metadata_values (asset_id, field_id, ord, value_text, value_int, value_real, updated_at)
                     SELECT asset_id, {fid}, 0, {t}, {i}, NULL, {now} FROM tmp_ids WHERE true
                     ON CONFLICT (asset_id, field_id, ord) DO UPDATE SET value_text = excluded.value_text, value_int = excluded.value_int, value_real = excluded.value_real, updated_at = excluded.updated_at"
                )
            };
            tx.execute_batch(&up(1, "'값'", "NULL")).unwrap();
            tx.execute_batch(&up(10, "NULL", "20261120")).unwrap();
            tx.execute_batch(&up(16, "NULL", "1601")).unwrap();
            tx.execute_batch(&format!("UPDATE media_assets SET revision = revision + 1, updated_at = {now} WHERE asset_id IN (SELECT asset_id FROM tmp_ids)")).unwrap();
            tx.execute_batch(&format!(
                "INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at)
                 SELECT a.asset_id, a.revision, 'upsert', CASE WHEN t.seq < 1000 THEN 0 ELSE 1 END, {now}
                 FROM tmp_ids t JOIN media_assets a ON a.asset_id = t.asset_id"
            ))
            .unwrap();
            tx.commit().unwrap();
            tms.push(ms(t.elapsed()));
        }
        row("B 임시 테이블 + 집합 SQL", count, &tms);
    }
    // 지우기: 한 필드 값 삭제 + revision + outbox (10,000건)
    let mut tms = Vec::new();
    for run in 0..5 {
        let ids = pick_ids(r, n, 10_000);
        let now = 1_700_000_200_000i64 + run;
        let t = Instant::now();
        let tx = c.unchecked_transaction().unwrap();
        {
            let mut del = tx.prepare_cached("DELETE FROM asset_metadata_values WHERE asset_id = ?1 AND field_id = ?2").unwrap();
            let mut bump = tx.prepare_cached("UPDATE media_assets SET revision = revision + 1, updated_at = ?2 WHERE asset_id = ?1 RETURNING revision").unwrap();
            let mut ob = tx.prepare_cached("INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at) VALUES (?1,?2,'upsert',1,?3)").unwrap();
            for id in &ids {
                del.execute(params![id, 10]).unwrap();
                let rev: i64 = bump.query_row(params![id, now], |r| r.get(0)).unwrap();
                ob.execute(params![id, rev, now]).unwrap();
            }
        }
        tx.commit().unwrap();
        tms.push(ms(t.elapsed()));
    }
    row("지우기 1필드 (자산별 반복)", 10_000, &tms);
    let ob: i64 = c.query_row("SELECT count(*) FROM search_outbox", [], |r| r.get(0)).unwrap();
    println!("(outbox 누적 행 {ob})");
}

fn row(name: &str, count: usize, tms: &[f64]) {
    let mut s = tms.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "| {name} | {count} | {} | {} | {:.0} |",
        tms.len(),
        tms.iter().map(|x| format!("{x:.0}")).collect::<Vec<_>>().join(" / "),
        s[s.len() / 2]
    );
}

/// 필드 삭제 (field 3: 값 ~80만 행) — 대상 조회 + CASCADE + revision + outbox 를 한 트랜잭션에서.
fn field_delete(c: &Connection, path: &Path) {
    println!("\n[필드 삭제: field_id=3 (D1 절차: 대상 조회 -> 필드 삭제(CASCADE) -> revision++ -> outbox priority 2)]");
    let now = 1_700_000_300_000i64;
    let t = Instant::now();
    let tx = c.unchecked_transaction().unwrap();
    let t1 = Instant::now();
    let ids: Vec<i64> = {
        let mut st = tx.prepare("SELECT asset_id FROM asset_metadata_values WHERE field_id = 3").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect()
    };
    let d_sel = t1.elapsed();
    let t2 = Instant::now();
    tx.execute("DELETE FROM metadata_fields WHERE field_id = 3", []).unwrap();
    let d_del = t2.elapsed();
    let t3 = Instant::now();
    tx.execute_batch("CREATE TEMP TABLE del_ids (asset_id INTEGER PRIMARY KEY)").unwrap();
    {
        let mut ins = tx.prepare("INSERT INTO del_ids VALUES (?1)").unwrap();
        for id in &ids {
            ins.execute([id]).unwrap();
        }
    }
    tx.execute(&format!("UPDATE media_assets SET revision = revision + 1, updated_at = {now} WHERE asset_id IN (SELECT asset_id FROM del_ids)"), []).unwrap();
    let d_rev = t3.elapsed();
    let t4 = Instant::now();
    tx.execute(&format!("INSERT INTO search_outbox (asset_id, revision, event_type, priority, created_at) SELECT a.asset_id, a.revision, 'upsert', 2, {now} FROM del_ids d JOIN media_assets a USING (asset_id)"), []).unwrap();
    let d_ob = t4.elapsed();
    tx.commit().unwrap();
    println!(
        "  대상 {} 건: 조회 {:.0} ms, 필드 DELETE(CASCADE) {:.0} ms, revision++ {:.0} ms, outbox {:.0} ms, 전체 트랜잭션 {:.0} ms",
        ids.len(), ms(d_sel), ms(d_del), ms(d_rev), ms(d_ob), ms(t.elapsed())
    );
    let left: i64 = c.query_row("SELECT count(*) FROM asset_metadata_values WHERE field_id = 3", [], |r| r.get(0)).unwrap();
    let fk: i64 = c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r.get(0)).unwrap();
    println!("  삭제 후 field 3 값 잔여 {left} 행, foreign_key_check 위반 {fk}, db_file={:.0} MB", file_mb(path));
}
