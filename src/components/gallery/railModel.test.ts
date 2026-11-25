// Categories: Happy (T-ui-rail-30, 33, 36, 38, 39) / Boundary (T-ui-rail-31, 32, 34, 37)
import type { TFunction } from "i18next"
import { describe, expect, it } from "vitest"
import type { SearchIndex } from "@/lib/ipc-contract"
import { buildTicks, handleFraction, railLabel, sectionAtFraction, slotAt, slotFraction } from "./railModel"

const t = ((k: string) => ({ "gallery.noDate": "날짜 없음", "gallery.rail.etc": "기타" })[k] ?? k) as unknown as TFunction

function monthIndex(years: number, none = false): { index: SearchIndex; fracs: number[] } {
  const sections: SearchIndex["sections"] = []
  for (let y = 0; y < years; y++) for (let m = 1; m <= 12; m++) sections.push({ key: `${1980 + y}-${String(m).padStart(2, "0")}`, label: null, count: 10 })
  if (none) sections.push({ key: "none", label: null, count: 10 })
  const total = sections.length * 10
  return { index: { total, generation: 1, kind: "month", sections }, fracs: sections.map((_, i) => (i * 10) / total) }
}

describe("buildTicks", () => {
  it("T-ui-rail-30 [Happy] year label on each year's first section; the other months become dots", () => {
    // Arrange
    const { index, fracs } = monthIndex(3)
    // Act
    const ticks = buildTicks(index, fracs, 600, t)
    // Assert
    expect(ticks.labels.map((l) => l.text)).toEqual(["1980", "1981", "1982"])
    expect(ticks.dots).toHaveLength(36 - 3)
  })

  it("T-ui-rail-31 [Boundary] crowded years are thinned so consecutive labels keep >= 16px, and 260+ sections draw no dots", () => {
    // Arrange: 40년 × 12월 = 480 구간을 500px 에
    const { index, fracs } = monthIndex(40)
    // Act
    const ticks = buildTicks(index, fracs, 500, t)
    // Assert
    expect(ticks.labels.length).toBeGreaterThan(1)
    expect(ticks.labels.length).toBeLessThan(40)
    for (let i = 1; i < ticks.labels.length; i++) expect(ticks.labels[i].y - ticks.labels[i - 1].y).toBeGreaterThanOrEqual(16)
    expect(ticks.dots).toHaveLength(0)
  })

  it("T-ui-rail-32 [Boundary] kind none, no index or zero height produce nothing", () => {
    // Arrange
    const none: SearchIndex = { total: 5, generation: 1, kind: "none", sections: [{ key: "all", label: null, count: 5 }] }
    // Act + Assert
    expect(buildTicks(none, [0], 500, t)).toEqual({ labels: [], dots: [] })
    expect(buildTicks(undefined, [], 500, t)).toEqual({ labels: [], dots: [] })
    expect(buildTicks(monthIndex(2).index, [0], 0, t)).toEqual({ labels: [], dots: [] })
  })

  it("T-ui-rail-33 [Happy] railLabel maps every initial key family", () => {
    // Act + Assert
    expect(["sym", "a", "z", "etc", "etc_end", "ㄱ", "ㅎ"].map((k) => railLabel("initial", k, t))).toEqual(["#", "A", "Z", "기타", "기타", "ㄱ", "ㅎ"])
    expect(railLabel("month", "2024-03", t)).toBe("2024")
    expect(railLabel("month", "none", t)).toBe("날짜 없음")
    expect(railLabel("none", "all", t)).toBeNull()
  })
})

describe("sectionAtFraction", () => {
  it("T-ui-rail-34 [Boundary] picks the last section at or before the position, clamping both ends", () => {
    // Arrange
    const fracs = [0, 0.25, 0.5, 0.75]
    // Act + Assert
    expect([-1, 0, 0.24, 0.25, 0.9, 1].map((f) => sectionAtFraction(fracs, f))).toEqual([0, 0, 0, 1, 3, 3])
    expect(sectionAtFraction([], 0.5)).toBe(0)
  })
})

describe("name index slots", () => {
  const keys = (list: string[]): SearchIndex => ({ total: list.length * 10, generation: 1, kind: "initial", sections: list.map((key) => ({ key, label: null, count: 10 })) })

  it("T-ui-rail-36 [Happy] every section gets an evenly spaced slot regardless of its count", () => {
    // Arrange: 첫 구간이 압도적으로 커도 간격은 같다
    const index = keys(["sym", "a", "b", "ㄱ", "ㅎ"])
    index.sections[0].count = 100_000
    // Act
    const ys = buildTicks(index, [], 502, t).labels.map((l) => l.y)
    // Assert
    expect(ys).toHaveLength(5)
    const gaps = ys.slice(1).map((y, i) => y - ys[i])
    expect(Math.max(...gaps) - Math.min(...gaps)).toBeLessThan(1e-9)
    expect(ys[0]).toBe(6)
    expect(ys[4]).toBe(496)
  })

  it("T-ui-rail-37 [Boundary] a crowded track keeps the first section of each script group and the last, then thins the rest by spacing", () => {
    // Arrange: 구간 12개를 40px 트랙에 (칸 간격 3px)
    const list = ["sym", "a", "b", "c", "d", "e", "etc", "ㄱ", "ㄴ", "ㄷ", "ㅎ", "etc_end"]
    // Act
    const texts = buildTicks(keys(list), [], 40, t).labels.map((l) => l.text)
    // Assert
    for (const must of ["#", "A", "기타", "ㄱ"]) expect(texts).toContain(must)
    expect(texts[texts.length - 1]).toBe("기타") // etc_end
    expect(texts.length).toBeLessThan(list.length)
    expect(buildTicks(keys(["a"]), [], 40, t).labels).toHaveLength(1) // 구간 하나
  })

  it("T-ui-rail-38 [Happy] slotAt picks the nearest slot and is the inverse of slotFraction", () => {
    // Act + Assert
    for (let i = 0; i < 16; i++) expect(slotAt(16, slotFraction(16, i, 600), 600)).toBe(i)
    expect([-5, 0, 1, 5].map((f) => slotAt(16, f, 600))).toEqual([0, 0, 15, 15])
    expect(slotAt(1, 0.7, 600)).toBe(0)
  })

  it("T-ui-rail-39 [Happy] handleFraction interpolates inside a section and is monotonic across sections", () => {
    // Arrange: 구간 [0,10) [10,110) [110,120)
    const starts = [0, 10, 110, 120]
    // Act
    const hs = [0, 5, 10, 60, 109, 110, 119].map((g) => handleFraction(starts, g, 600))
    // Assert
    for (let i = 1; i < hs.length; i++) expect(hs[i]).toBeGreaterThanOrEqual(hs[i - 1])
    expect(hs[0]).toBeCloseTo(slotFraction(3, 0, 600))
    expect(hs[2]).toBeCloseTo(slotFraction(3, 1, 600))
    expect(hs[3]).toBeCloseTo((slotFraction(3, 1, 600) + slotFraction(3, 2, 600)) / 2)
    expect(hs[5]).toBeCloseTo(slotFraction(3, 2, 600))
    expect(handleFraction([0], 0, 600)).toBe(0)
  })
})
