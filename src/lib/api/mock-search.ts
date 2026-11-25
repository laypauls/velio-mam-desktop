import {
  MAX_PAGE_LIMIT,
  type CommandError,
  type SearchFilters,
  type SearchIndex,
  type SearchIndexRequest,
  type SearchPage,
  type CapturedAtFallback,
  type SearchRequest,
  type SortSpec,
} from "@/lib/ipc-contract"
import { monthKey } from "@/lib/format"
import { toSummary, type MockDb, type Rec } from "./mock-data"

export function cmdError(code: CommandError["code"], message: string): CommandError {
  return { code, message }
}

const CURSOR_PREFIX = "mam1:"

/** `generation` 은 hybrid 커서만 싣는다 — 인덱스 재구축·모델 교체 뒤 옛 커서를 거부하는 백엔드 동작 모사. */
export function encodeCursor(offset: number, generation?: number): string {
  return btoa(`${CURSOR_PREFIX}${offset}${generation === undefined ? "" : `:g${generation}`}`)
}

function parseCursor(cursor: string): { offset: number; generation: number | null } {
  let raw: string
  try {
    raw = atob(cursor)
  } catch {
    throw cmdError("invalid_input", "invalid cursor")
  }
  const m = /^mam1:(\d+)(?::g(\d+))?$/.exec(raw)
  if (!m) throw cmdError("invalid_input", "invalid cursor")
  return { offset: Number(m[1]), generation: m[2] === undefined ? null : Number(m[2]) }
}

export function decodeCursor(cursor: string): number {
  return parseCursor(cursor).offset
}

/** 촬영일 대체 규칙 적용 후 날짜(ms) — 백엔드 `captured_or_modified` 필드 모사. "none" 이면 촬영일이 없는 자산은 null */
function capturedOf(r: Rec, fallback: CapturedAtFallback): number | null {
  return r.captured ?? (fallback === "modified" ? r.modified : null)
}

function matchesFilters(db: MockDb, r: Rec, f: SearchFilters, fallback: CapturedAtFallback): boolean {
  if (r.status !== "active") return false
  if (f.mediaTypes.length > 0 && !f.mediaTypes.includes(r.type)) return false
  if (r.avail === "offline" && !f.includeOffline) return false
  if (r.avail === "missing" && !f.includeMissing) return false
  if (f.favoritesOnly && !r.fav) return false
  if (f.ratingMin !== null && r.rating < f.ratingMin) return false
  if (f.ratingMax !== null && r.rating > f.ratingMax) return false
  const when = capturedOf(r, fallback)
  if (f.capturedFrom !== null && (when === null || when < f.capturedFrom)) return false
  if (f.capturedTo !== null && (when === null || when > f.capturedTo)) return false
  if (f.sizeMin !== null && r.size < f.sizeMin) return false
  if (f.sizeMax !== null && r.size > f.sizeMax) return false
  if (f.durationMinMs !== null && (r.dur === null || r.dur < f.durationMinMs)) return false
  if (f.durationMaxMs !== null && (r.dur === null || r.dur > f.durationMaxMs)) return false
  // 숨긴 관리 폴더는 폴더 범위·컬렉션 보기가 아니면 제외한다 (백엔드와 같은 규칙)
  if (!f.folder && f.collectionId === null && db.folders.some((x) => x.folderId === r.folderId && x.hidden === true)) return false
  if (f.folder) {
    if (r.folderId !== f.folder.folderId) return false
    const rel = f.folder.relPath
    if (rel !== null) {
      const inside = r.rel === rel || (f.folder.recursive && r.rel.startsWith(`${rel}/`))
      if (!inside) return false
    }
  }
  if (f.collectionId !== null && !db.collections.get(f.collectionId)?.members.has(r.id)) return false
  for (const t of f.tagIds) if (!r.tags.includes(t)) return false
  return true
}

function relevance(db: MockDb, r: Rec, q: string): number {
  if (!q) return 0
  if (r.nameLower === q) return 4
  if (r.nameLower.startsWith(q)) return 3
  if (r.nameLower.includes(q)) return 2
  return r.tags.some((t) => db.tags.get(t)?.toLowerCase().includes(q)) ? 1 : 0.5
}

function sortKey(db: MockDb, sort: SortSpec, q: string, fallback: CapturedAtFallback, cid: number | null): (r: Rec) => number | string | null {
  switch (sort.field) {
    case "capturedAt":
      return (r) => capturedOf(r, fallback)
    case "modifiedAt":
      return (r) => r.modified
    case "name":
      return (r) => r.nameLower
    case "size":
      return (r) => r.size
    case "rating":
      return (r) => r.rating
    case "duration":
      return (r) => r.dur
    case "relevance":
      return (r) => relevance(db, r, q)
    case "collectionOrder": {
      // members Set 의 삽입 순서 = 컬렉션 순서 (reorder 는 Set 을 다시 만든다)
      const order = new Map<number, number>()
      const members = db.collections.get(cid ?? -1)?.members
      if (members) for (const id of members) order.set(id, order.size)
      return (r) => order.get(r.id) ?? null
    }
  }
}

interface CacheEntry {
  key: string
  version: number
  ids: Int32Array
}
const caches = new WeakMap<MockDb, CacheEntry>()

function matchingIds(db: MockDb, req: SearchRequest, fallback: CapturedAtFallback): Int32Array {
  const key = JSON.stringify([req.query, req.filters, req.sort, fallback])
  const hit = caches.get(db)
  if (hit && hit.key === key && hit.version === db.version) return hit.ids

  const q = req.query.trim().toLowerCase()
  const matched: Rec[] = []
  for (const r of db.recs) {
    if (!matchesFilters(db, r, req.filters, fallback)) continue
    if (q) {
      const inName = r.nameLower.includes(q)
      const inMemo = r.memo !== "" && r.memo.toLowerCase().includes(q)
      const inTag = !inName && !inMemo && r.tags.some((t) => db.tags.get(t)?.toLowerCase().includes(q))
      if (!inName && !inMemo && !inTag) continue
    }
    matched.push(r)
  }

  if (req.sort.field === "collectionOrder" && req.filters.collectionId === null) throw cmdError("invalid_input", "sort")
  const keyOf = sortKey(db, req.sort, q, fallback, req.filters.collectionId)
  const dir = req.sort.direction === "asc" ? 1 : -1
  matched.sort((a, b) => {
    const ka = keyOf(a)
    const kb = keyOf(b)
    if (ka === null || kb === null) {
      if (ka === kb) return a.id - b.id
      return ka === null ? 1 : -1 // null 은 방향과 무관하게 뒤로
    }
    if (ka < kb) return -dir
    if (ka > kb) return dir
    return a.id - b.id
  })

  const ids = Int32Array.from(matched, (r) => r.id)
  caches.set(db, { key, version: db.version, ids })
  return ids
}

/**
 * `mode: "hybrid"` 는 임베딩 모델이 준비되지 않았으면 keyword 로 처리하고 `degraded` 를 표시한다.
 * 준비됐으면 페이지 안에서 인접 항목 순서를 바꿔 "다른 순위" 를 흉내 낸다 (mock).
 */
export function runSearch(
  db: MockDb,
  req: SearchRequest,
  semanticReady = false,
  generation: { value: number } = { value: 0 },
  fallback: CapturedAtFallback = "modified",
): SearchPage {
  if (!Number.isInteger(req.limit) || req.limit < 1 || req.limit > MAX_PAGE_LIMIT) {
    throw cmdError("invalid_input", `limit must be 1..${MAX_PAGE_LIMIT}`)
  }
  const hybridReq = req.mode === "hybrid" && req.query.trim() !== ""
  const backward = req.direction === "backward"
  if (req.cursor !== null && req.at !== undefined) throw cmdError("invalid_input", "cursor and at are mutually exclusive")
  if (backward && req.cursor === null) throw cmdError("invalid_input", "backward paging requires a cursor")
  if (req.at !== undefined && (!Number.isInteger(req.at) || req.at < 0)) throw cmdError("invalid_input", "at must be a non-negative integer")
  const parsed = req.cursor === null ? { offset: 0, generation: null } : parseCursor(req.cursor)
  if (hybridReq && parsed.generation !== null && parsed.generation !== generation.value) {
    throw cmdError("invalid_input", "stale cursor")
  }
  const t0 = performance.now()
  const ids = matchingIds(db, req, fallback)
  if (parsed.offset > ids.length) throw cmdError("invalid_input", "invalid cursor")
  // 커서는 경계 위치다: forward 는 그 위치부터, backward 는 그 위치 앞 limit 건. at 이 총계 이상이면 빈 페이지
  const start = backward
    ? Math.max(0, parsed.offset - req.limit)
    : Math.min(req.at ?? parsed.offset, ids.length)
  const end = backward ? parsed.offset : Math.min(start + req.limit, ids.length)
  const items = []
  for (let i = start; i < end; i++) items.push(toSummary(db.recs[ids[i] - 1]))
  const hybrid = req.mode === "hybrid" && req.query.trim() !== ""
  if (hybrid && semanticReady) {
    for (let i = 0; i + 1 < items.length; i += 2) [items[i], items[i + 1]] = [items[i + 1], items[i]]
  }
  const cursorGeneration = hybridReq ? generation.value : undefined
  return {
    items,
    total: ids.length,
    nextCursor: items.length > 0 && end < ids.length ? encodeCursor(end, cursorGeneration) : null,
    prevCursor: items.length > 0 && start > 0 ? encodeCursor(start, cursorGeneration) : null,
    startOffset: start,
    generation: generation.value,
    totalIsEstimate: false,
    tookMs: Math.max(0, Math.round(performance.now() - t0)),
    ...(hybrid && !semanticReady ? { degraded: true } : {}),
  }
}

const INITIAL_KEYS_AFTER_Z = ["ㄱ", "ㄴ", "ㄷ", "ㄹ", "ㅁ", "ㅂ", "ㅅ", "ㅇ", "ㅈ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ"] as const
/** 한글 음절 초성 인덱스(0..18) → 구간 (ㄲ·ㄸ·ㅃ·ㅆ·ㅉ 는 앞 자음에 합침) — 백엔드 `name_initial_bounds` 와 같은 규칙 */
const CHOSEONG_SLOT = [0, 0, 1, 2, 2, 3, 4, 5, 5, 6, 6, 7, 8, 8, 9, 10, 11, 12, 13]

/** 이름 정렬 키(NFC + 소문자)의 첫 글자 → 구간 키 (백엔드 `search_index` 의 `initial` 규칙 모사). */
export function initialKey(name: string): string {
  const cp = name.normalize("NFC").toLowerCase().codePointAt(0)
  if (cp === undefined || cp < 0x61) return "sym"
  if (cp <= 0x7a) return String.fromCharCode(cp)
  if (cp < 0xac00) return "etc"
  if (cp <= 0xd7a3) return INITIAL_KEYS_AFTER_Z[CHOSEONG_SLOT[Math.floor((cp - 0xac00) / 588)]]
  return "etc_end"
}

/** `search_index` mock — 월은 로컬 시간대(`monthKey` 와 같은 구간), 없는 값은 "none"(항상 맨 뒤). */
export function runSearchIndex(
  db: MockDb,
  req: SearchIndexRequest,
  generation: { value: number } = { value: 0 },
  fallback: CapturedAtFallback = "modified",
): SearchIndex {
  const ids = matchingIds(db, { ...req, limit: 1, cursor: null }, fallback)
  const field = req.sort.field
  const kind: SearchIndex["kind"] =
    field === "capturedAt" || field === "modifiedAt" ? "month" : field === "name" ? "initial" : "none"
  const keyOf = (r: Rec): string => {
    if (kind === "initial") return initialKey(r.nameLower)
    if (kind === "none") return "all"
    const ms = field === "capturedAt" ? capturedOf(r, fallback) : r.modified
    return ms === null ? "none" : monthKey(ms)
  }
  // matchingIds 가 이미 정렬 순서이고 구간은 정렬 순서에서 연속이다 — 첫 등장 순서가 곧 구간 순서
  const sections: SearchIndex["sections"] = []
  const byKey = new Map<string, SearchIndex["sections"][number]>()
  for (const id of ids) {
    const key = keyOf(db.recs[id - 1])
    let s = byKey.get(key)
    if (!s) {
      s = { key, label: null, count: 0 }
      byKey.set(key, s)
      sections.push(s)
    }
    s.count++
  }
  return { total: ids.length, generation: generation.value, kind, sections }
}
