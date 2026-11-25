//! 인덱스 열기·생성, upsert(revision 가드)·삭제·commit.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use tantivy::collector::TopDocs;
use tantivy::directory::{Directory, MmapDirectory};
use tantivy::query::TermQuery;
use tantivy::schema::IndexRecordOption;
use tantivy::{Index, IndexReader, IndexWriter, Order, ReloadPolicy, Term};

use crate::analyzer;
use crate::doc::{AssetDoc, BuildFlags};
use crate::error::{Result, SearchError};
use crate::korean;
use crate::retry_dir::{RetryDirectory, RetryPolicy};
use crate::schema::{Fields, build_schema};

/// 어떤 파일명 부분 일치 보조 필드를 만들지와 writer 자원.
#[derive(Clone, Debug)]
pub struct IndexOptions {
    /// B 안: `file_name_ngram` 을 채운다.
    pub build_ngram: bool,
    /// C 안: `file_name_raw` 를 채운다.
    pub build_raw: bool,
    pub writer_heap_bytes: usize,
    pub writer_threads: usize,
}

impl IndexOptions {
    /// A 안(기준선) — 보조 필드 없음.
    pub fn strategy_a() -> Self {
        Self {
            build_ngram: false,
            build_raw: false,
            writer_heap_bytes: 200_000_000,
            writer_threads: 2,
        }
    }
    pub fn strategy_b() -> Self {
        Self {
            build_ngram: true,
            ..Self::strategy_a()
        }
    }
    pub fn strategy_c() -> Self {
        Self {
            build_raw: true,
            ..Self::strategy_a()
        }
    }
    /// 비교·테스트용: B·C 보조 필드를 모두 만든다.
    pub fn all() -> Self {
        Self {
            build_ngram: true,
            build_raw: true,
            ..Self::strategy_a()
        }
    }
}

/// `upsert` 의 결과.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpsertOutcome {
    Applied,
    /// 이미 더 높은 revision 이 색인(또는 commit 대기)돼 있어 무시했다.
    Stale {
        indexed_revision: u64,
    },
}

struct WriterState {
    writer: IndexWriter,
    /// `rollback` 이 실패해 writer 의 내부 상태(디렉터리 잠금 포함)를 믿을 수 없다.
    /// 이 상태에서는 writer 를 다시 건드리지 않고 모든 쓰기를 `WriterPoisoned` 로 거부한다.
    poisoned: bool,
    /// commit 전 이 배치에서 반영한 asset_id → revision (searcher 가 아직 못 보는 문서의 가드용).
    pending: HashMap<u64, u64>,
}

pub struct SearchIndex {
    pub(crate) reader: IndexReader,
    pub(crate) fields: Fields,
    pub(crate) options: IndexOptions,
    state: Mutex<WriterState>,
}

impl SearchIndex {
    /// 디렉터리의 인덱스를 열고 없으면 만든다. 쓰기 잠금을 즉시 획득하므로 같은 디렉터리를
    /// 두 번 열면 두 번째는 `Tantivy(LockFailure)` 오류다. 손상된 메타·스키마 불일치도 오류로 반환한다.
    /// 이전에 다른 형식·분석기 버전·전략 옵션으로 만든 인덱스는 `IndexMismatch` (재구축 필요).
    pub fn open_or_create(dir: &Path, options: IndexOptions) -> Result<Self> {
        Self::open_or_create_with_retry(dir, options, RetryPolicy::default(), |d| Box::new(d))
    }

    /// 재시도 정책과 그 **아래** 계층(`MmapDirectory` 를 감싸는 실패 주입 등)을 지정한다.
    /// Windows 의 일시 접근 거부는 [`RetryDirectory`] 가 흡수한다. 테스트·진단용.
    #[doc(hidden)]
    pub fn open_or_create_with_retry(
        dir: &Path,
        options: IndexOptions,
        policy: RetryPolicy,
        below: impl FnOnce(MmapDirectory) -> Box<dyn Directory>,
    ) -> Result<Self> {
        Self::open_with(dir, options, |d| {
            Box::new(RetryDirectory::new(below(d), policy))
        })
    }

    /// 테스트용: 재시도 계층 **위**를 감싸 I/O 실패를 주입한다 (재시도로 복구되지 않는
    /// 지속 실패 모사 — 주입한 실패가 그대로 commit 실패가 된다). 일반 코드는 `open_or_create`.
    #[doc(hidden)]
    pub fn open_or_create_with_directory(
        dir: &Path,
        options: IndexOptions,
        wrap: impl FnOnce(RetryDirectory) -> Box<dyn Directory>,
    ) -> Result<Self> {
        Self::open_with(dir, options, |d| {
            wrap(RetryDirectory::new(Box::new(d), RetryPolicy::default()))
        })
    }

    fn open_with(
        dir: &Path,
        options: IndexOptions,
        wrap: impl FnOnce(MmapDirectory) -> Box<dyn Directory>,
    ) -> Result<Self> {
        korean::ensure_ready()?;
        std::fs::create_dir_all(dir)?;
        crate::meta::check_or_write(dir, &options)?;
        let (schema, fields) = build_schema();
        let index = Index::open_or_create(wrap(MmapDirectory::open(dir)?), schema)?;
        analyzer::register(&index);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        let writer = index
            .writer_with_num_threads(options.writer_threads.max(1), options.writer_heap_bytes)?;
        Ok(Self {
            reader,
            fields,
            options,
            state: Mutex::new(WriterState {
                writer,
                poisoned: false,
                pending: HashMap::new(),
            }),
        })
    }

    fn state(&self) -> MutexGuard<'_, WriterState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn flags(&self) -> BuildFlags {
        BuildFlags {
            ngram: self.options.build_ngram,
            raw: self.options.build_raw,
        }
    }

    /// commit 된 인덱스에서 `asset_id` 의 revision. 살아 있는 문서가 여러 개면 최댓값.
    /// 비용: 세그먼트마다 용어 사전 조회 1회 + fast field 읽기 (문서 저장소는 읽지 않는다).
    fn committed_revision(&self, asset_id: u64) -> Result<Option<u64>> {
        let searcher = self.reader.searcher();
        let q = TermQuery::new(
            Term::from_field_u64(self.fields.asset_id, asset_id),
            IndexRecordOption::Basic,
        );
        let top = searcher.search(
            &q,
            &TopDocs::with_limit(1).order_by_u64_field("revision", Order::Desc),
        )?;
        Ok(top.first().and_then(|(rev, _)| *rev))
    }

    /// `asset_id` 문서를 교체한다 (delete_term + add). **revision 가드**: 이미 색인된 revision 이
    /// 더 높으면 아무것도 바꾸지 않고 `Stale` 을 반환한다. 같은 revision 은 멱등 재적용이다.
    /// 가드는 commit 된 인덱스(searcher)와 이번 배치의 대기 맵을 함께 본다.
    pub fn upsert(&self, doc: &AssetDoc) -> Result<UpsertOutcome> {
        let tdoc = doc.to_tantivy(&self.fields, self.flags())?;
        let mut st = self.state();
        Self::ensure_usable(&st)?;
        let known = match st.pending.get(&doc.asset_id) {
            Some(r) => Some(*r),
            None => self.committed_revision(doc.asset_id)?,
        };
        if let Some(indexed_revision) = known.filter(|r| *r > doc.revision) {
            return Ok(UpsertOutcome::Stale { indexed_revision });
        }
        Self::replace(&mut st, &self.fields, doc, tdoc)?;
        Ok(UpsertOutcome::Applied)
    }

    /// 가드 없이 교체한다 — **빈 인덱스에 SQLite 로부터 재구축**할 때만 쓴다 (조회 비용 없음).
    pub fn upsert_unguarded(&self, doc: &AssetDoc) -> Result<()> {
        let tdoc = doc.to_tantivy(&self.fields, self.flags())?;
        let mut st = self.state();
        Self::ensure_usable(&st)?;
        Self::replace(&mut st, &self.fields, doc, tdoc)
    }

    fn ensure_usable(st: &WriterState) -> Result<()> {
        if st.poisoned {
            Err(SearchError::WriterPoisoned)
        } else {
            Ok(())
        }
    }

    /// 쓰기 도중 실패한 배치를 버린다: Tantivy `rollback` 은 새 `IndexWriter`(새 색인 스레드)를
    /// 만들고 마지막 commit 상태로 되돌린다. 실패한 commit/add 뒤의 옛 writer 는 색인 스레드가
    /// 죽어 있어 이후 commit 이 `Ok` 를 돌려주면서 문서를 잃기 때문이다. 대기 맵도 비운다.
    /// rollback 자체가 실패하면(내부에서 디렉터리 잠금을 이미 꺼낸 상태라 재시도하면 패닉한다)
    /// writer 를 더 이상 건드리지 않고 poisoned 로 표시한다 — 다시 열어야 복구된다.
    fn discard_batch(st: &mut WriterState) {
        st.pending.clear();
        if st.writer.rollback().is_err() {
            st.poisoned = true;
        }
    }

    fn replace(
        st: &mut WriterState,
        f: &Fields,
        doc: &AssetDoc,
        tdoc: tantivy::TantivyDocument,
    ) -> Result<()> {
        st.writer
            .delete_term(Term::from_field_u64(f.asset_id, doc.asset_id));
        if let Err(e) = st.writer.add_document(tdoc) {
            Self::discard_batch(st);
            return Err(e.into());
        }
        st.pending.insert(doc.asset_id, doc.revision);
        Ok(())
    }

    /// `asset_id` 문서를 삭제한다.
    ///
    /// **알려진 한계**: 삭제 후에는 인덱스에 revision 이 남지 않으므로, 늦게 도착한 낮은 revision 의
    /// upsert 가 삭제된 자산을 되살릴 수 있다(tombstone 을 두지 않기로 한 결정). 삭제를 가로지르는
    /// 역전 방지의 정본은 SQLite `index_progress.applied_revision`(삭제 후에도 남음)이며 P2 색인
    /// 워커가 적용한다. 이 인덱스의 가드는 방어 계층일 뿐이다.
    pub fn delete(&self, asset_id: u64) -> Result<()> {
        let mut st = self.state();
        Self::ensure_usable(&st)?;
        st.writer
            .delete_term(Term::from_field_u64(self.fields.asset_id, asset_id));
        st.pending.insert(asset_id, 0); // 0 = 삭제 표시 — commit 전 재색인은 어떤 revision 이든 허용
        Ok(())
    }

    /// 대기 중인 변경을 commit 하고 검색에 반영한다.
    ///
    /// **실패 계약**: `Err` 이면 이번 배치(마지막 성공 commit 이후의 모든 upsert/delete)는
    /// **버려진 것**이다 — 호출자는 배치를 다시 적용(upsert/delete 후 commit)해야 하고, Outbox 이벤트는
    /// commit 이 `Ok` 를 반환한 뒤에만 완료 처리해야 한다. `add_document` 가 `Tantivy` 오류를
    /// 반환한 경우도 같다. rollback 마저 실패하면 이후 모든 쓰기는 `WriterPoisoned` 이며 인덱스를
    /// 다시 열어야 한다.
    pub fn commit(&self) -> Result<()> {
        let mut st = self.state();
        Self::ensure_usable(&st)?;
        if let Err(e) = st.writer.commit() {
            Self::discard_batch(&mut st);
            return Err(e.into());
        }
        // reload 가 실패하면 searcher 는 아직 옛 상태이므로 가드 맵을 유지한다 (성공 뒤에 비움).
        self.reader.reload()?;
        st.pending.clear();
        Ok(())
    }

    /// 마지막 commit 이후의 미커밋 변경은 버리고, 진행 중인 백그라운드 병합이 끝나기를 기다린 뒤
    /// writer 를 닫는다(디렉터리 잠금·파일 핸들 해제). 재구축 직후 디렉터리를 교체하기 전에 쓴다.
    /// writer 가 poisoned 이면 병합 대기 없이 닫고 `WriterPoisoned` 를 반환한다.
    pub fn close_waiting_for_merges(self) -> Result<()> {
        let st = self.state.into_inner().unwrap_or_else(|e| e.into_inner());
        if st.poisoned {
            return Err(SearchError::WriterPoisoned);
        }
        st.writer.wait_merging_threads()?;
        Ok(())
    }

    /// commit 된 살아 있는 문서 수.
    pub fn num_docs(&self) -> u64 {
        self.reader.searcher().num_docs()
    }
}
