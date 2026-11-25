// Categories: Happy (T-ui-capfb-01, 02) / Business rule (T-ui-capfb-01~04) / Boundary (T-ui-capfb-03)
import { describe, expect, it } from "vitest"
import type { AssetSummary, SearchRequest } from "@/lib/ipc-contract"
import { DEFAULT_FILTERS } from "@/stores/search"
import { createMockApi } from "./mock"

const req = (over: Partial<SearchRequest> = {}): SearchRequest => ({
  query: "",
  filters: DEFAULT_FILTERS,
  sort: { field: "capturedAt", direction: "asc" },
  limit: 200,
  cursor: null,
  ...over,
})

const effective = (a: AssetSummary) => a.capturedAt ?? a.modifiedAt

describe("mock search: capturedAtFallback", () => {
  it("T-ui-capfb-01 [Business rule] default is 'modified': captured-date sort orders by capturedAt ?? modifiedAt", async () => {
    // Arrange
    const api = createMockApi({ count: 3000 })
    expect((await api.getSettings()).capturedAtFallback).toBe("modified")
    // Act
    const page = await api.searchAssets(req())
    // Assert
    const times = page.items.map(effective)
    expect(times).toEqual([...times].sort((a, b) => a - b))
    expect(page.items.some((i) => i.capturedAt === null)).toBe(true) // 대체가 실제로 섞여 있는 데이터
  })

  it("T-ui-capfb-02 [Business rule] 'none': assets without a captured date sort last in both directions", async () => {
    // Arrange
    const api = createMockApi({ count: 3000 })
    await api.updateSettings({ capturedAtFallback: "none" })
    const index = await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort: req().sort })
    const nulls = index.sections.find((s) => s.key === "none")!.count
    // Act
    const asc = await api.searchAssets(req({ at: index.total - nulls, limit: nulls }))
    const desc = await api.searchAssets(req({ sort: { field: "capturedAt", direction: "desc" }, at: index.total - nulls, limit: nulls }))
    const first = await api.searchAssets(req({ sort: { field: "capturedAt", direction: "desc" } }))
    // Assert
    expect(asc.items.every((i) => i.capturedAt === null)).toBe(true)
    expect(desc.items.every((i) => i.capturedAt === null)).toBe(true)
    expect(first.items.every((i) => i.capturedAt !== null)).toBe(true)
  })

  it("T-ui-capfb-03 [Boundary] month sections: a 'none' section exists only when the fallback is 'none'", async () => {
    // Arrange
    const api = createMockApi({ count: 3000 })
    const sort = { field: "capturedAt", direction: "desc" } as const
    const withFallback = await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort })
    await api.updateSettings({ capturedAtFallback: "none" })
    // Act
    const without = await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort })
    // Assert
    expect(withFallback.sections.some((s) => s.key === "none")).toBe(false)
    const none = without.sections.find((s) => s.key === "none")
    expect(none).toBeDefined()
    expect(without.sections[without.sections.length - 1]?.key).toBe("none")
    expect(without.total).toBe(withFallback.total) // 정렬·구간만 달라지고 결과 수는 같다
  })

  it("T-ui-capfb-04 [Business rule] captured range filter: 'modified' keeps undated assets, 'none' excludes them (cache is keyed by the setting)", async () => {
    // Arrange
    const api = createMockApi({ count: 3000 })
    const filters = { ...DEFAULT_FILTERS, capturedFrom: 0 }
    // Act
    const all = (await api.searchAssets(req({ limit: 1 }))).total
    const modified = await api.searchAssets(req({ filters, limit: 1 }))
    await api.updateSettings({ capturedAtFallback: "none" })
    const none = await api.searchAssets(req({ filters, limit: 1 }))
    // Assert
    expect(modified.total).toBe(all)
    expect(none.total).toBeLessThan(modified.total)
  })
})
