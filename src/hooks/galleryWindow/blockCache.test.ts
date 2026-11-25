// Categories: Happy / Boundary / Business rule / Concurrency (stale epoch, generation)
import { describe, expect, it } from "vitest"
import type { AssetSummary, SearchPage } from "@/lib/ipc-contract"
import { BlockCache, INFINITE_BLOCK_SIZE, INFINITE_MAX_BLOCKS, PAGED_MAX_BLOCKS, neededBlocks, type PlannedRequest } from "./blockCache"

const item = (id: number): AssetSummary => ({
  assetId: id, fileName: `f${id}.jpg`, title: null, mediaType: "image", availability: "online", width: 1, height: 1,
  durationMs: null, fileSize: 1, capturedAt: null, modifiedAt: 0, favorite: false, rating: 0, thumbState: "done", aiState: "none", revision: 1,
})

/** 블록 b 의 응답 (전체 total 건 중) */
function pageFor(b: number, bs: number, total: number, over: Partial<SearchPage> = {}): SearchPage {
  const start = b * bs
  const end = Math.min(total, start + bs)
  const items = Array.from({ length: Math.max(0, end - start) }, (_, i) => item(start + i))
  return {
    items, total, nextCursor: end < total ? `n${end}` : null, prevCursor: start > 0 ? `p${start}` : null,
    startOffset: start, generation: 1, totalIsEstimate: false, tookMs: 1, ...over,
  }
}

/** 계획된 요청을 즉시 완료 처리 */
function settle(c: BlockCache, reqs: PlannedRequest[], total: number) {
  for (const r of reqs) c.accept(r, pageFor(r.blockIndex, c.blockSize, total))
}
function fill(c: BlockCache, blocks: number[], total: number) {
  for (const b of blocks) {
    c.setNeeded([b])
    const reqs = c.plan([], total)
    settle(c, reqs, total)
  }
}
const seeded = (s: number) => () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)

describe("neededBlocks", () => {
  it("T-ui-win-01 [Happy] covers visible blocks plus overscan, clamped to total", () => {
    expect(neededBlocks(250, 420, 100, 1000, 1)).toEqual([1, 2, 3, 4, 5])
  })
  it("T-ui-win-02 [Boundary] clamps at both ends and for empty/unknown total", () => {
    expect(neededBlocks(0, 10, 100, 1000, 1)).toEqual([0, 1])
    expect(neededBlocks(990, 999, 100, 1000, 1)).toEqual([8, 9])
    expect(neededBlocks(0, 0, 100, 0, 1)).toEqual([0])
    expect(neededBlocks(5, 5, 100, null, 0)).toEqual([0])
  })
})

describe("BlockCache eviction", () => {
  it.each([
    ["infinite", INFINITE_BLOCK_SIZE, INFINITE_MAX_BLOCKS],
    ["paged", 50, PAGED_MAX_BLOCKS],
  ])("T-ui-win-03 [Boundary] %s: block count never exceeds the cap over random jumps", (_n, bs, max) => {
    const total = 1_000_000
    const c = new BlockCache({ blockSize: bs, maxBlocks: max })
    c.setGeneration(1)
    const rnd = seeded(7)
    for (let i = 0; i < 300; i++) {
      const first = Math.floor(rnd() * (total - 500))
      const span = max === PAGED_MAX_BLOCKS ? 0 : Math.floor(rnd() * 400)
      c.setNeeded(neededBlocks(first, first + span, bs, total, 1))
      for (let guard = 0; guard < 20; guard++) {
        const reqs = c.plan([], total)
        if (!reqs.length) break
        expect(c.inFlightCount).toBeLessThanOrEqual(3)
        settle(c, reqs, total)
        expect(c.size).toBeLessThanOrEqual(max)
      }
    }
    expect(c.size).toBeLessThanOrEqual(max)
  })

  it("T-ui-win-04 [Business rule] evicts the block farthest from the center first", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 3 })
    c.setGeneration(1)
    fill(c, [0, 1, 2], 10_000)
    expect(c.cachedIndexes()).toEqual([0, 1, 2])
    // Act: 중심을 2 로 옮기고 블록 3 을 받는다 → 0 이 가장 멀다
    c.setNeeded([2, 3])
    settle(c, c.plan([], 10_000), 10_000)
    expect(c.cachedIndexes()).toEqual([1, 2, 3])
  })

  it("T-ui-win-05 [Business rule] needed blocks are never evicted", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 4 })
    c.setGeneration(1)
    fill(c, [10, 11, 12, 13], 10_000)
    c.setNeeded([10, 11])
    c.plan([], 10_000)
    // 먼 블록 3 개를 더 받아 상한을 압박해도 필요 블록은 남는다
    for (const b of [50, 51, 52]) c.accept({ blockIndex: b, kind: "at", cursor: null, at: b * 100, epoch: c.currentEpoch }, pageFor(b, 100, 10_000))
    expect(c.has(10)).toBe(true)
    expect(c.has(11)).toBe(true)
    expect(c.size).toBeLessThanOrEqual(4)
  })

  it("T-ui-win-06 [Boundary] more needed blocks than the cap keeps only the nearest to the center", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 3 })
    c.setGeneration(1)
    c.setNeeded([0, 1, 2, 3, 4])
    // Act: 중심(2)의 블록 먼저, 이웃은 커서를 얻을 수 있도록 기다린다
    const first = c.plan([], 10_000)
    expect(first.map((r) => r.blockIndex)).toEqual([2])
    settle(c, first, 10_000)
    const second = c.plan([], 10_000)
    // Assert: 필요 블록은 1~3 으로 제한되고 0, 4 는 요청하지 않는다
    expect(second.map((r) => r.blockIndex).sort()).toEqual([1, 3])
    expect(second.map((r) => r.kind).sort()).toEqual(["next", "prev"])
  })
})

describe("BlockCache request planning", () => {
  it("T-ui-win-07 [Happy] block 0 uses a null cursor, next uses previous block's nextCursor", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([0])
    const [r0] = c.plan([], 1000)
    expect(r0).toMatchObject({ blockIndex: 0, kind: "next", cursor: null })
    settle(c, [r0], 1000)
    c.setNeeded([1])
    const [r1] = c.plan([], 1000)
    expect(r1).toMatchObject({ blockIndex: 1, kind: "next", cursor: "n100" })
  })

  it("T-ui-win-08 [Happy] backward with the following block's prevCursor when only that one is cached", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    fill(c, [5], 1000) // at 로 받음
    c.setNeeded([4])
    const [r] = c.plan([], 1000)
    expect(r).toMatchObject({ blockIndex: 4, kind: "prev", cursor: "p500" })
  })

  it("T-ui-win-09 [Happy] seeks with at = blockIndex * blockSize when no neighbor is cached", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([7])
    expect(c.plan([], 1000)).toEqual([{ blockIndex: 7, kind: "at", cursor: null, at: 700, epoch: c.currentEpoch }])
  })

  it("T-ui-win-10 [Business rule] waits for an in-flight neighbor instead of seeking, and honours max 3 in flight", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([3, 4, 5, 6, 7])
    const reqs = c.plan([], 10_000)
    expect(reqs).toHaveLength(3)
    expect(c.inFlightCount).toBe(3)
    expect(c.plan([], 10_000)).toHaveLength(0)
  })

  it("T-ui-win-11 [Boundary] never plans blocks past the total, and nothing for an empty result", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([0, 1, 2])
    expect(c.plan([], 150).map((r) => r.blockIndex).sort()).toEqual([0, 1])
    const d = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    d.setNeeded([0])
    expect(d.plan([], 0)).toEqual([])
  })

  it("T-ui-win-12 [Business rule] prefetch happens once, only after needed blocks are loaded", () => {
    const c = new BlockCache({ blockSize: 50, maxBlocks: 2 })
    c.setGeneration(1)
    c.setNeeded([2])
    const first = c.plan([3], 1000)
    expect(first.map((r) => r.blockIndex)).toEqual([2]) // 필요 블록이 비어 있는 동안은 미리 받지 않는다
    settle(c, first, 1000)
    const second = c.plan([3], 1000)
    expect(second.map((r) => r.blockIndex)).toEqual([3])
    settle(c, second, 1000)
    // 3 이 축출된 뒤에도 다시 미리 받지 않는다
    c.setNeeded([9, 10])
    settle(c, c.plan([], 1000), 1000)
    settle(c, c.plan([], 1000), 1000)
    expect(c.has(3)).toBe(false)
    c.setNeeded([2])
    settle(c, c.plan([3], 1000), 1000)
    expect(c.plan([3], 1000)).toEqual([])
  })
})

describe("BlockCache responses", () => {
  it("T-ui-win-13 [Concurrency] a response from a stale epoch is dropped", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([0])
    const [r] = c.plan([], 1000)
    c.reset() // 질의·정렬 변경
    expect(c.accept(r, pageFor(0, 100, 1000))).toBe("stale")
    expect(c.size).toBe(0)
    c.fail(r) // 오래된 실패는 새 진행 표시를 건드리지 않는다
    c.setNeeded([0])
    expect(c.plan([], 1000)).toHaveLength(1)
    c.fail(r)
    expect(c.inFlightCount).toBe(1)
  })

  it("T-ui-win-14 [Concurrency] a different response generation is reported and nothing is stored", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([0])
    const [r] = c.plan([], 1000)
    expect(c.accept(r, pageFor(0, 100, 1000, { generation: 2 }))).toBe("generation")
    expect(c.size).toBe(0)
  })

  it("T-ui-win-15 [Concurrency] setGeneration with a new value invalidates all blocks and in-flight responses", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    fill(c, [0, 1], 1000)
    c.setNeeded([2])
    const [r] = c.plan([], 1000)
    expect(c.setGeneration(1)).toBe(false)
    expect(c.setGeneration(2)).toBe(true)
    expect(c.size).toBe(0)
    expect(c.accept(r, pageFor(2, 100, 1000, { generation: 2 }))).toBe("stale")
  })

  it("T-ui-win-16 [Boundary] startOffset not equal to the block start (data shifted) is treated as a generation change", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([3])
    const [r] = c.plan([], 1000)
    expect(c.accept(r, pageFor(3, 100, 1000, { startOffset: 305 }))).toBe("generation")
    expect(c.size).toBe(0)
  })

  it("T-ui-win-17 [Boundary] an empty page where the index promised items counts as a generation change", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([3])
    const [r] = c.plan([], 1000)
    expect(c.accept(r, pageFor(3, 100, 1000, { items: [], startOffset: 1000 }))).toBe("generation")
  })

  it("T-ui-win-18 [Happy] getItem resolves global positions inside cached blocks only; last block may be partial", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    fill(c, [2], 250)
    expect(c.getItem(200)?.assetId).toBe(200)
    expect(c.getItem(249)?.assetId).toBe(249)
    expect(c.getItem(250)).toBeUndefined()
    expect(c.getItem(150)).toBeUndefined()
    expect(c.getItem(-1)).toBeUndefined()
    expect(c.loadedCount()).toBe(50)
  })

  it("T-ui-win-19 [External] fail releases the in-flight slot so the block can be planned again", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    c.setNeeded([0])
    const [r] = c.plan([], 1000)
    expect(c.plan([], 1000)).toHaveLength(0)
    c.fail(r)
    expect(c.plan([], 1000)).toHaveLength(1)
  })
})

describe("BlockCache stale-while-revalidate and lookups", () => {
  it("T-ui-gwin-80 [Business rule] markStale keeps blocks readable, re-requests only the needed stale blocks, and accept clears the mark", () => {
    // Arrange: 블록 0~2 보관, 필요한 것은 1 하나
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    fill(c, [0, 1, 2], 1000)
    c.setNeeded([1])
    expect(c.plan([], 1000)).toHaveLength(0)
    // Act
    c.markStale()
    const reqs = c.plan([], 1000)
    // Assert: 보이는 동안 데이터는 그대로, 필요한 블록만 다시 요청 (0·2 는 필요하지 않아 요청하지 않는다)
    expect(c.getItem(150)?.assetId).toBe(150)
    expect(reqs.map((r) => r.blockIndex)).toEqual([1])
    settle(c, reqs, 1000)
    expect(c.plan([], 1000)).toHaveLength(0)
  })

  it("T-ui-gwin-81 [Concurrency] reset clears stale marks; a stale block still counts as loaded for prefetch gating", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    fill(c, [0], 1000)
    c.markStale()
    c.reset()
    c.setNeeded([0])
    // reset 뒤에는 보관 블록이 없으므로 일반 요청 1건 (stale 표시가 남아 중복 요청되지 않는다)
    expect(c.plan([], 1000)).toHaveLength(1)
    expect(c.plan([], 1000)).toHaveLength(0)
  })

  it("T-ui-gwin-82 [Happy] positionOf / loadedIds find held items by id in global order and return null / skip the rest", () => {
    const c = new BlockCache({ blockSize: 100, maxBlocks: 12 })
    c.setGeneration(1)
    fill(c, [3, 1], 450)
    expect(c.positionOf(100)).toBe(100)
    expect(c.positionOf(349)).toBe(349)
    expect(c.positionOf(5)).toBeNull()
    const ids = c.loadedIds()
    expect(ids).toHaveLength(200)
    expect(ids[0]).toBe(100)
    expect(ids[100]).toBe(300)
  })
})
