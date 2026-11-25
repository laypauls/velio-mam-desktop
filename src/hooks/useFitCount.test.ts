// Categories: Happy (T-ui-fit-01) / Boundary (T-ui-fit-02..05)
import { describe, expect, it } from "vitest"
import { calcVisibleCount } from "./useFitCount"

const ITEM = 32
const GAP = 8
const needed = (n: number) => n * ITEM + (n - 1) * GAP

describe("calcVisibleCount", () => {
  it("T-ui-fit-01 [Happy] everything fits: all items inline, no overflow slot", () => {
    // Arrange + Act + Assert
    expect(calcVisibleCount(1000, 6, ITEM, GAP)).toBe(6)
    expect(calcVisibleCount(1000, 4, ITEM, GAP)).toBe(4)
  })

  it("T-ui-fit-02 [Boundary] width of 0 means not measured yet: show all", () => {
    expect(calcVisibleCount(0, 6, ITEM, GAP)).toBe(6)
  })

  it("T-ui-fit-03 [Boundary] exactly fits all vs one pixel short", () => {
    expect(calcVisibleCount(needed(6), 6, ITEM, GAP)).toBe(6)
    // 1px 부족 → ⋯ 한 칸 확보: 5칸 중 4개 + ⋯
    expect(calcVisibleCount(needed(6) - 1, 6, ITEM, GAP)).toBe(4)
  })

  it("T-ui-fit-04 [Boundary] very narrow: only the ⋯ button remains", () => {
    expect(calcVisibleCount(ITEM, 6, ITEM, GAP)).toBe(0)
    expect(calcVisibleCount(10, 6, ITEM, GAP)).toBe(0)
  })

  it("T-ui-fit-05 [Boundary] original actions on (6) vs off (4) change the threshold; empty list is 0", () => {
    const width = needed(5)
    expect(calcVisibleCount(width, 4, ITEM, GAP)).toBe(4)
    expect(calcVisibleCount(width, 6, ITEM, GAP)).toBe(4)
    expect(calcVisibleCount(width, 0, ITEM, GAP)).toBe(0)
  })
})
