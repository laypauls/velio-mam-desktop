// Categories: Happy (T-ui-ord-01, 02) / Boundary (T-ui-ord-03, 05) / Business rule (T-ui-ord-04) / External (T-ui-ord-06)
import { describe, expect, it, vi } from "vitest"
import { computeTarget, planReorder, type ReorderContext } from "./reorderTarget"

/** 위치 i 의 자산 id = 100 + i (전체 `total` 개). `loaded` 밖의 위치는 idAt 이 비동기로 채운다 */
function ctx(selectedPositions: number[], total = 10, loaded: (p: number) => boolean = () => true): ReorderContext & { fetched: number[] } {
  const fetched: number[] = []
  return {
    selected: new Set(selectedPositions.map((p) => 100 + p)),
    positionOf: (id) => (loaded(id - 100) ? id - 100 : null),
    idAt: (p) => {
      if (p < 0 || p >= total) return undefined
      if (loaded(p)) return 100 + p
      fetched.push(p)
      return Promise.resolve(100 + p)
    },
    total,
    fetched,
  }
}

describe("computeTarget", () => {
  it("T-ui-ord-01 [Happy] start / end ignore neighbours; up goes before the item right above the selection", async () => {
    // Arrange
    const c = ctx([4, 5])
    // Act + Assert
    expect(await computeTarget("start", c)).toEqual({ kind: "start" })
    expect(await computeTarget("end", c)).toEqual({ kind: "end" })
    expect(await computeTarget("up", c)).toEqual({ kind: "before", assetId: 103 })
  })

  it("T-ui-ord-02 [Happy] down puts the block after the next item: before the one after it, or at the end", async () => {
    // Arrange + Act + Assert: 선택 [4,5] → 6 뒤 = 7 앞
    expect(await computeTarget("down", ctx([4, 5]))).toEqual({ kind: "before", assetId: 107 })
    // 6 이 마지막 항목이면 맨 뒤
    expect(await computeTarget("down", ctx([4, 5], 7))).toEqual({ kind: "end" })
  })

  it("T-ui-ord-03 [Boundary] up at the very top and down at the very bottom have no target", async () => {
    // Arrange + Act + Assert
    expect(await computeTarget("up", ctx([0, 1]))).toBeNull()
    expect(await computeTarget("down", ctx([8, 9]))).toBeNull()
  })

  it("T-ui-ord-04 [Business rule] a sparse selection is gathered: up skips selected gaps, down passes the nearest unselected item", async () => {
    // Arrange: 선택 [3,5]
    // Act + Assert: 첫 선택(3) 바로 위(2) 앞으로 → 3,5 가 2 앞에 모인다
    expect(await computeTarget("up", ctx([3, 5]))).toEqual({ kind: "before", assetId: 102 })
    // 마지막 선택(5) 다음 비선택(6) 뒤 = 7 앞
    expect(await computeTarget("down", ctx([3, 5]))).toEqual({ kind: "before", assetId: 107 })
    // 인접 선택이 이어져도 비선택까지 건너뛴다: [3,4,5] 위쪽은 2
    expect(await computeTarget("up", ctx([3, 4, 5]))).toEqual({ kind: "before", assetId: 102 })
  })

  it("T-ui-ord-05 [Boundary] nothing known about the selection yields null for up/down but start/end still work", async () => {
    // Arrange: 선택 항목이 메모리에 없다
    const c = ctx([4], 10, () => false)
    // Act + Assert
    expect(await computeTarget("up", c)).toBeNull()
    expect(await computeTarget("start", c)).toEqual({ kind: "start" })
  })

  it("T-ui-ord-06 [External] an unloaded neighbour is resolved through idAt (fetch) and the result uses it", async () => {
    // Arrange: 위치 3 은 메모리에 없다 (선택 4 위)
    const idAt = vi.fn((p: number) => (p === 3 ? Promise.resolve(103) : 100 + p))
    const c: ReorderContext = {
      selected: new Set([104]),
      positionOf: (id) => (id === 104 ? 4 : null),
      idAt,
      total: 10,
    }
    // Act
    const target = await computeTarget("up", c)
    // Assert
    expect(idAt).toHaveBeenCalledWith(3)
    expect(target).toEqual({ kind: "before", assetId: 103 })
  })

  it("T-ui-ord-06b [External] a neighbour that the fetch cannot resolve is skipped; none left means null", async () => {
    // Arrange: 위쪽 두 칸이 비어 있다(undefined)
    const c: ReorderContext = { selected: new Set([102]), positionOf: () => 2, idAt: () => undefined, total: 10 }
    // Act + Assert
    expect(await computeTarget("up", c)).toBeNull()
  })
})

describe("planReorder reveal position", () => {
  it("T-ui-ord-11 [Happy] start → 0, end → last position, up → the neighbour's old position", async () => {
    // Arrange
    const c = ctx([4, 5])
    // Act + Assert
    expect((await planReorder("start", c))?.reveal).toBe(0)
    expect((await planReorder("end", c))?.reveal).toBe(9)
    expect((await planReorder("up", c))?.reveal).toBe(3)
  })

  it("T-ui-ord-12 [Happy] down: the block lands right before the item after the next one, so reveal = that position minus the moved items ahead of it", async () => {
    // Arrange + Act: 선택 [4,5] → 7 앞, 블록 첫 항목의 새 위치 = 7 - 2
    const plan = await planReorder("down", ctx([4, 5]))
    // Assert
    expect(plan).toEqual({ target: { kind: "before", assetId: 107 }, reveal: 5 })
    // 뒤에 하나만 남으면 맨 뒤
    expect((await planReorder("down", ctx([4, 5], 7)))?.reveal).toBe(6)
  })

  it("T-ui-ord-13 [External] an off-screen (unloaded) target still yields a reveal position from the fetched neighbour", async () => {
    // Arrange: 위쪽 이웃(위치 1)이 메모리에 없다
    const c = ctx([2], 10, (p) => p !== 1)
    // Act
    const plan = await planReorder("up", c)
    // Assert
    expect(c.fetched).toContain(1)
    expect(plan).toEqual({ target: { kind: "before", assetId: 101 }, reveal: 1 })
  })

  it("T-ui-ord-14 [Boundary] no plan when nothing can move", async () => {
    // Arrange + Act + Assert
    expect(await planReorder("up", ctx([0]))).toBeNull()
  })
})
