import type { AssetSummary, SearchPage } from "@/lib/ipc-contract"

/**
 * 갤러리 창(window) 블록 캐시 — React 없는 순수 로직.
 * 결과를 `blockSize` 건 블록으로 잘라 최대 `maxBlocks` 개만 보관한다 (100만 건을 올리지 않는다).
 */

/** 동시 진행 요청 상한 */
export const MAX_IN_FLIGHT = 3
/** 무한 스크롤: 100건 × 12 블록 */
export const INFINITE_BLOCK_SIZE = 100
export const INFINITE_MAX_BLOCKS = 12
/** 페이지 모드: 현재 + 다음(미리 받기) + 직전 */
export const PAGED_MAX_BLOCKS = 3

export interface Block {
  index: number
  items: AssetSummary[]
  nextCursor: string | null
  prevCursor: string | null
}

export type RequestKind = "next" | "prev" | "at"

/** `search_assets` 로 보낼 요청 계획 (limit = blockSize) */
export interface PlannedRequest {
  blockIndex: number
  kind: RequestKind
  /** next/prev 는 기준 블록의 커서(블록 0 의 next 는 null), at 은 null */
  cursor: string | null
  /** at 일 때 전역 시작 위치 */
  at?: number
  /** 이 요청을 계획한 시점의 요청 세대 — `accept`/`fail` 에 되돌려 준다 */
  epoch: number
}

export type AcceptResult =
  /** 저장됨 */
  | "stored"
  /** 질의·정렬·모드가 바뀐 뒤 온 옛 응답 — 버림 */
  | "stale"
  /** 색인 세대가 다르거나 시작 위치가 어긋남 — 호출자가 캐시를 비우고 `search_index` 를 다시 받아야 한다 */
  | "generation"

export interface BlockCacheOptions {
  blockSize: number
  maxBlocks: number
  maxInFlight?: number
}

/** 보이는 항목 구간 [first, last](포함) + 앞뒤 `overscan` 블록 → 필요한 블록 번호 (총계 안으로 제한) */
export function neededBlocks(first: number, last: number, blockSize: number, total: number | null, overscan = 1): number[] {
  const lastBlock = total === null ? Number.MAX_SAFE_INTEGER : Math.max(0, Math.ceil(total / blockSize) - 1)
  const a = Math.max(0, Math.floor(Math.max(0, first) / blockSize) - overscan)
  const b = Math.min(lastBlock, Math.floor(Math.max(first, last) / blockSize) + overscan)
  const out: number[] = []
  for (let i = a; i <= b; i++) out.push(i)
  return out
}

export class BlockCache {
  readonly blockSize: number
  readonly maxBlocks: number
  readonly maxInFlight: number
  /** 현재 색인 세대 — `setGeneration` 전에는 null */
  generation: number | null = null
  /** 최근 응답의 부가 정보 */
  meta: { totalIsEstimate: boolean; degraded: boolean } = { totalIsEstimate: false, degraded: false }
  private epoch = 0
  private readonly blocks = new Map<number, Block>()
  private readonly inFlight = new Set<number>()
  private readonly prefetched = new Set<number>()
  /** 내용이 바뀌었을 수 있는(편집·분석 반영) 블록 — 화면에는 계속 보이고, 필요하면 제자리에서 다시 받는다 */
  private readonly stale = new Set<number>()
  private needed: number[] = []

  constructor(opts: BlockCacheOptions) {
    this.blockSize = opts.blockSize
    this.maxBlocks = opts.maxBlocks
    this.maxInFlight = opts.maxInFlight ?? MAX_IN_FLIGHT
  }

  get size(): number {
    return this.blocks.size
  }
  get inFlightCount(): number {
    return this.inFlight.size
  }
  get currentEpoch(): number {
    return this.epoch
  }
  has(index: number): boolean {
    return this.blocks.has(index)
  }
  cachedIndexes(): number[] {
    return [...this.blocks.keys()].sort((x, y) => x - y)
  }
  loadedCount(): number {
    let n = 0
    for (const b of this.blocks.values()) n += b.items.length
    return n
  }

  /** 보관 중인 블록에서 assetId 의 전역 위치 — 없으면 null (블록은 최대 `maxBlocks` 개라 선형 탐색이 싸다) */
  positionOf(assetId: number): number | null {
    for (const b of this.blocks.values()) {
      const i = b.items.findIndex((a) => a.assetId === assetId)
      if (i >= 0) return b.index * this.blockSize + i
    }
    return null
  }

  /** 보관 중인 항목의 assetId — 전역 위치 순서 (블록 사이가 비어 있을 수 있다) */
  loadedIds(): number[] {
    const out: number[] = []
    for (const b of [...this.blocks.values()].sort((x, y) => x.index - y.index)) for (const a of b.items) out.push(a.assetId)
    return out
  }

  getItem(g: number): AssetSummary | undefined {
    if (g < 0) return undefined
    const bi = Math.floor(g / this.blockSize)
    return this.blocks.get(bi)?.items[g - bi * this.blockSize]
  }

  /** 질의·정렬·모드 변경: 모든 블록·진행 요청을 무효화(진행 중 응답은 `stale` 로 버려진다) */
  reset(): void {
    this.epoch++
    this.clearData()
  }

  /** 같은 세대에서 결과가 갱신됐을 수 있다(즐겨찾기·제목·태그 편집) — 보관 블록을 지우지 않고 필요한 것만 다시 받게 표시한다 */
  markStale(): void {
    for (const b of this.blocks.keys()) this.stale.add(b)
  }

  /** 색인 세대 갱신. 이전 세대와 다르면 보관 데이터를 비운다 (epoch 도 올려 진행 응답을 버린다) */
  setGeneration(generation: number): boolean {
    const changed = this.generation !== null && this.generation !== generation
    this.generation = generation
    if (changed) this.reset()
    return changed
  }

  /** 필요 블록 지정 — 상한(`maxBlocks`)을 넘으면 중심에 가까운 것만 필요로 취급. 필요하지 않은 먼 블록을 축출한다. */
  setNeeded(blocks: number[]): void {
    const center = blocks.length ? blocks.reduce((s, b) => s + b, 0) / blocks.length : 0
    const sorted = [...blocks].sort((x, y) => Math.abs(x - center) - Math.abs(y - center) || x - y)
    this.needed = sorted.slice(0, this.maxBlocks)
    this.evict()
  }

  /**
   * 요청 계획: 필요한데 없는 블록(중심에서 가까운 순) + 미리 받을 블록(필요 블록이 모두 채워진 뒤, 블록당 한 번).
   * 이웃 블록을 받는 중이면 커서를 얻을 수 있으므로 비싼 seek 대신 기다린다.
   * 반환한 요청은 이미 진행 중으로 표시된다.
   */
  plan(prefetch: number[] = [], total: number | null = null): PlannedRequest[] {
    const out: PlannedRequest[] = []
    const inRange = (b: number) => b >= 0 && (total === null || b * this.blockSize < total)
    const want = this.needed.filter((b) => inRange(b) && (!this.blocks.has(b) || this.stale.has(b)) && !this.inFlight.has(b))
    const stillNeeded = this.needed.some((b) => inRange(b) && !this.blocks.has(b))
    if (!stillNeeded) {
      for (const b of prefetch) {
        if (inRange(b) && !this.blocks.has(b) && !this.inFlight.has(b) && !this.prefetched.has(b)) want.push(b)
      }
    }
    for (const b of want) {
      if (this.inFlight.size >= this.maxInFlight) break
      const req = this.requestFor(b)
      if (req === "wait") continue
      if (!this.needed.includes(b)) this.prefetched.add(b)
      this.inFlight.add(b)
      out.push(req)
    }
    return out
  }

  private requestFor(b: number): PlannedRequest | "wait" {
    const epoch = this.epoch
    if (b === 0) return { blockIndex: 0, kind: "next", cursor: null, epoch }
    const before = this.blocks.get(b - 1)
    if (before?.nextCursor) return { blockIndex: b, kind: "next", cursor: before.nextCursor, epoch }
    const after = this.blocks.get(b + 1)
    if (after?.prevCursor) return { blockIndex: b, kind: "prev", cursor: after.prevCursor, epoch }
    if (this.inFlight.has(b - 1) || this.inFlight.has(b + 1)) return "wait"
    return { blockIndex: b, kind: "at", cursor: null, at: b * this.blockSize, epoch }
  }

  /** 응답 반영. 오래된 요청·세대 불일치·시작 위치 어긋남을 구분해 돌려준다. */
  accept(req: PlannedRequest, page: SearchPage): AcceptResult {
    if (req.epoch !== this.epoch) return "stale"
    this.inFlight.delete(req.blockIndex)
    if (this.generation !== null && page.generation !== this.generation) return "generation"
    // 데이터가 밀렸다 (스캔 중 삽입·삭제) — 세대가 같아도 블록 경계가 맞지 않으면 신뢰할 수 없다
    if (page.items.length > 0 && page.startOffset !== req.blockIndex * this.blockSize) return "generation"
    // 빈 페이지: 색인이 있다고 한 위치가 비었다 — 총계가 줄어든 경쟁 상태이므로 색인을 다시 받는다
    if (page.items.length === 0) return "generation"
    this.stale.delete(req.blockIndex)
    this.blocks.set(req.blockIndex, {
      index: req.blockIndex,
      items: page.items,
      nextCursor: page.nextCursor,
      prevCursor: page.prevCursor,
    })
    this.meta = { totalIsEstimate: page.totalIsEstimate, degraded: page.degraded === true }
    this.evict()
    return "stored"
  }

  /** 실패한 요청의 진행 표시를 해제 (오래된 요청이면 무시) */
  fail(req: PlannedRequest): void {
    if (req.epoch === this.epoch) this.inFlight.delete(req.blockIndex)
  }

  private clearData(): void {
    this.blocks.clear()
    this.inFlight.clear()
    this.prefetched.clear()
    this.stale.clear()
  }

  private evict(): void {
    if (this.blocks.size <= this.maxBlocks) return
    const keep = new Set(this.needed)
    const center = this.needed.length ? this.needed.reduce((s, b) => s + b, 0) / this.needed.length : 0
    const victims = [...this.blocks.keys()]
      .filter((b) => !keep.has(b))
      .sort((x, y) => Math.abs(y - center) - Math.abs(x - center) || y - x)
    for (const v of victims) {
      if (this.blocks.size <= this.maxBlocks) break
      this.blocks.delete(v)
      this.stale.delete(v)
    }
  }
}
