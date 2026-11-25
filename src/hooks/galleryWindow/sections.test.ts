// Categories: Happy / Boundary (empty, partial last row, range start mid-section, y at exact boundaries)
import { describe, expect, it } from "vitest"
import { buildSectionModel, itemAt, itemSpan, offsetOfItem, offsetOfSection, sectionAt, sectionsInRange, visibleItemRange, type SectionCount } from "./sections"

const P = { cols: 4, grouped: true, headerH: 40, rowH: 100 }
// 10 + 5 + 8 = 23 항목
const S: SectionCount[] = [
  { key: "2026-03", count: 10 },
  { key: "2026-02", count: 5 },
  { key: "none", count: 8 },
]

describe("buildSectionModel", () => {
  it("T-ui-win-30 [Happy] computes rows, heights and cumulative startY per section (last partial row counts)", () => {
    const m = buildSectionModel(S, 0, 23, P)
    expect(m.sections.map((s) => [s.key, s.start, s.count, s.rows, s.startY, s.height, s.continued])).toEqual([
      ["2026-03", 0, 10, 3, 0, 340, false],
      ["2026-02", 10, 5, 2, 340, 240, false],
      ["none", 15, 8, 2, 580, 240, false],
    ])
    expect(m.totalHeight).toBe(820)
  })

  it("T-ui-win-31 [Happy] changing cols only re-accumulates heights", () => {
    const m = buildSectionModel(S, 0, 23, { ...P, cols: 10 })
    expect(m.sections.map((s) => s.rows)).toEqual([1, 1, 1])
    expect(m.totalHeight).toBe(3 * 140)
  })

  it("T-ui-win-32 [Boundary] page range starting mid-section marks continued and clips counts", () => {
    // 페이지 = [7, 17): 1구간 3건(이어짐) + 2구간 5건 + 3구간 2건
    const m = buildSectionModel(S, 7, 17, P)
    expect(m.sections.map((s) => [s.sectionIndex, s.start, s.count, s.continued])).toEqual([
      [0, 7, 3, true],
      [1, 10, 5, false],
      [2, 15, 2, false],
    ])
  })

  it("T-ui-win-33 [Boundary] a range starting exactly at a section start is not continued; sections outside are skipped", () => {
    const m = buildSectionModel(S, 10, 15, P)
    expect(m.sections).toHaveLength(1)
    expect(m.sections[0]).toMatchObject({ sectionIndex: 1, start: 10, count: 5, continued: false })
  })

  it("T-ui-win-34 [Boundary] empty range or no sections gives an empty model", () => {
    expect(buildSectionModel(S, 5, 5, P).sections).toEqual([])
    expect(buildSectionModel([], 0, 0, P).totalHeight).toBe(0)
  })

  it("T-ui-win-35 [Happy] ungrouped makes a single header-less section over the whole range", () => {
    const m = buildSectionModel(S, 0, 23, { ...P, grouped: false })
    expect(m.sections).toHaveLength(1)
    expect(m.sections[0]).toMatchObject({ headerH: 0, rows: 6, height: 600, count: 23 })
  })

  it("T-ui-win-36 [Boundary] the 'none' (undated) section is an ordinary last section", () => {
    const m = buildSectionModel([{ key: "none", count: 3 }], 0, 3, P)
    expect(m.sections[0]).toMatchObject({ key: "none", rows: 1, height: 140 })
  })

  it("T-ui-win-37 [Boundary] stays O(sections) for 1,000,000 items", () => {
    const many: SectionCount[] = Array.from({ length: 400 }, (_, i) => ({ key: `k${i}`, count: 2500 }))
    const m = buildSectionModel(many, 0, 1_000_000, P)
    expect(m.sections).toHaveLength(400)
    expect(offsetOfItem(m, 999_999)).not.toBeNull()
  })
})

describe("lookups", () => {
  const m = buildSectionModel(S, 0, 23, P)

  it("T-ui-win-38 [Happy] offsetOfItem gives the row top under the header", () => {
    expect(offsetOfItem(m, 0)).toBe(40)
    expect(offsetOfItem(m, 3)).toBe(40)
    expect(offsetOfItem(m, 4)).toBe(140)
    expect(offsetOfItem(m, 10)).toBe(380) // 2구간 머리글(340) 아래
    expect(offsetOfItem(m, 22)).toBe(580 + 40 + 100)
    expect(offsetOfSection(m, 2)).toBe(580)
  })

  it("T-ui-win-39 [Boundary] offsetOfItem outside the range is null", () => {
    expect(offsetOfItem(m, -1)).toBeNull()
    expect(offsetOfItem(m, 23)).toBeNull()
    expect(offsetOfItem(buildSectionModel(S, 7, 17, P), 6)).toBeNull()
  })

  it("T-ui-win-40 [Happy] itemAt returns the first item of the row at y; header and out-of-range y clamp", () => {
    expect(itemAt(m, 50)).toBe(0)
    expect(itemAt(m, 150)).toBe(4)
    expect(itemAt(m, 10)).toBe(0) // 머리글 → 구간 첫 항목
    expect(itemAt(m, 345)).toBe(10)
    expect(itemAt(m, -50)).toBe(0)
    expect(itemAt(m, 99_999)).toBe(19) // 마지막 행의 첫 항목
    expect(itemAt(buildSectionModel(S, 0, 0, P), 0)).toBeNull()
    expect(sectionAt(m, 600)?.key).toBe("none")
  })

  it("T-ui-win-41 [Boundary] visibleItemRange expands to whole rows and respects exact boundaries", () => {
    expect(visibleItemRange(m, 0, 140)).toEqual([0, 3]) // 머리글 + 첫 행
    expect(visibleItemRange(m, 0, 141)).toEqual([0, 7]) // 둘째 행이 1px 보임
    expect(visibleItemRange(m, 140, 240)).toEqual([4, 7]) // 첫 행이 딱 끝난 지점에서 시작
    expect(visibleItemRange(m, 240, 340)).toEqual([8, 9]) // 마지막 부분 행 (2건)
    expect(visibleItemRange(m, 0, 10_000)).toEqual([0, 22])
  })

  it("T-ui-win-42 [Boundary] a viewport that only covers a header, or lies outside, has no new items", () => {
    expect(visibleItemRange(m, 345, 375)).toBeNull() // 다음 구간 머리글만 보임
    expect(visibleItemRange(m, 330, 375)).toEqual([8, 9]) // 앞 구간 끝 행 + 머리글
    expect(visibleItemRange(m, 9000, 9100)).toBeNull()
    expect(visibleItemRange(m, -100, 0)).toBeNull()
    expect(visibleItemRange(m, 100, 100)).toBeNull()
  })
})

describe("sectionsInRange / itemSpan", () => {
  it("T-ui-gwin-83 [Happy] picks only the sections overlapping the viewport", () => {
    const m = buildSectionModel(S, 0, 23, P)
    expect(sectionsInRange(m, 0, 10).map((s) => s.key)).toEqual(["2026-03"])
    expect(sectionsInRange(m, 330, 350).map((s) => s.key)).toEqual(["2026-03", "2026-02"])
    expect(sectionsInRange(m, 340, 580).map((s) => s.key)).toEqual(["2026-02"])
  })

  it("T-ui-gwin-84 [Boundary] out-of-model ranges and empty models give no sections", () => {
    const m = buildSectionModel(S, 0, 23, P)
    expect(sectionsInRange(m, 820, 900)).toEqual([])
    expect(sectionsInRange(m, -50, 0)).toEqual([])
    expect(sectionsInRange(buildSectionModel([], 0, 0, P), 0, 100)).toEqual([])
  })

  it("T-ui-gwin-85 [Business rule] the span of a section's first row includes its header; later rows start at their own top", () => {
    const m = buildSectionModel(S, 0, 23, P)
    expect(itemSpan(m, 0)).toEqual({ top: 0, bottom: 140 }) // 머리글 40 + 행 100
    expect(itemSpan(m, 4)).toEqual({ top: 140, bottom: 240 })
    expect(itemSpan(m, 10)).toEqual({ top: 340, bottom: 480 })
    expect(itemSpan(m, 99)).toBeNull()
  })
})
