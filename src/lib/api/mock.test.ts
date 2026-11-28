// Categories: Happy (T-ui-api-01) / Validation (T-ui-api-02) / Business rule (T-ui-api-03,04)
import { describe, expect, it } from "vitest"
import type { AssetSummary, CommandError, SearchPage, SearchRequest } from "@/lib/ipc-contract"
import { DEFAULT_FILTERS, DEFAULT_SORT } from "@/stores/search"
import { createMockApi } from "./mock"

const req = (over: Partial<SearchRequest> = {}): SearchRequest => ({
  query: "",
  filters: DEFAULT_FILTERS,
  sort: DEFAULT_SORT,
  limit: 50,
  cursor: null,
  ...over,
})

async function rejection(p: Promise<unknown>): Promise<CommandError> {
  try {
    await p
  } catch (e) {
    return e as CommandError
  }
  throw new Error("expected rejection")
}

const hasAvail = (page: SearchPage, a: AssetSummary["availability"]) => page.items.some((i) => i.availability === a)

describe("mock api search", () => {
  it("T-ui-api-01a [Happy] query matches filename case-insensitively and type filter narrows", async () => {
    // Arrange
    const api = createMockApi({ count: 3000 })
    const everything = await api.searchAssets(req())
    const first = everything.items[0]
    // Act
    const byName = await api.searchAssets(req({ query: first.fileName.toUpperCase() }))
    const images = await api.searchAssets(req({ filters: { ...DEFAULT_FILTERS, mediaTypes: ["image"] } }))
    // Assert
    expect(byName.items.map((i) => i.assetId)).toContain(first.assetId)
    expect(images.items.every((i) => i.mediaType === "image")).toBe(true)
    expect(images.total).toBeLessThan(everything.total)
  })

  it("T-ui-api-01b [Business rule] PL5 default shows offline, hides missing; includeMissing widens", async () => {
    // Arrange
    const api = createMockApi({ count: 3000 })
    // Act
    const dflt = await api.searchAssets(req({ limit: 200 }))
    const withMissing = await api.searchAssets(req({ limit: 200, filters: { ...DEFAULT_FILTERS, includeMissing: true } }))
    const noOffline = await api.searchAssets(req({ limit: 200, filters: { ...DEFAULT_FILTERS, includeOffline: false } }))
    // Assert
    expect(hasAvail(dflt, "offline")).toBe(true)
    expect(hasAvail(dflt, "missing")).toBe(false)
    expect(withMissing.total).toBeGreaterThan(dflt.total)
    expect(hasAvail(noOffline, "offline")).toBe(false)
    expect(noOffline.total).toBeLessThan(dflt.total)
  })

  it("T-ui-api-01c [Happy] sort orders results and cursor pages cover the total exactly once", async () => {
    // Arrange
    const api = createMockApi({ count: 1000 })
    const collect = async (sort: SearchRequest["sort"]) => {
      const items: AssetSummary[] = []
      let cursor: string | null = null
      do {
        const page: SearchPage = await api.searchAssets(req({ sort, limit: 200, cursor }))
        items.push(...page.items)
        cursor = page.nextCursor
      } while (cursor)
      return items
    }
    // 촬영일 없는 자산이 맨 뒤로 가는 것은 대체값 "빈값" 일 때의 규칙(기본 "수정일" 은 수정일 위치로 섞인다 — mock-captured-fallback.test.ts)
    await api.updateSettings({ capturedAtFallback: "none" })
    // Act
    const byName = await collect({ field: "name", direction: "asc" })
    const byCaptured = await collect({ field: "capturedAt", direction: "desc" })
    const total = (await api.searchAssets(req())).total
    // Assert
    expect(byName).toHaveLength(total)
    expect(new Set(byName.map((i) => i.assetId)).size).toBe(total)
    const names = byName.map((i) => i.fileName.toLowerCase())
    expect(names).toEqual([...names].sort())
    const dated = byCaptured.filter((i) => i.capturedAt !== null).map((i) => i.capturedAt as number)
    expect(dated).toEqual([...dated].sort((a, b) => b - a))
    const firstNull = byCaptured.findIndex((i) => i.capturedAt === null)
    expect(byCaptured.slice(firstNull).every((i) => i.capturedAt === null)).toBe(true)
  })

  it("T-ui-api-02 [Validation] rejects limit over 200 and malformed cursors with invalid_input", async () => {
    // Arrange
    const api = createMockApi({ count: 500 })
    // Act
    const tooBig = await rejection(api.searchAssets(req({ limit: 201 })))
    const zero = await rejection(api.searchAssets(req({ limit: 0 })))
    const garbage = await rejection(api.searchAssets(req({ cursor: "%%%not-base64" })))
    const wrongShape = await rejection(api.searchAssets(req({ cursor: btoa("offset=3") })))
    // Assert
    for (const e of [tooBig, zero, garbage, wrongShape]) expect(e.code).toBe("invalid_input")
  })

  it("T-ui-api-03 [Business rule] delete requires auth; wrong password rejected; exemption honours clock", async () => {
    // Arrange
    let clock = 1_000_000
    const api = createMockApi({ count: 500, password: "secret", now: () => clock })
    const page = await api.searchAssets(req({ filters: { ...DEFAULT_FILTERS, includeOffline: false } }))
    const id = page.items[0].assetId
    // Act + Assert
    expect((await rejection(api.deleteOriginals([id]))).code).toBe("auth_required")
    expect((await rejection(api.verifyPassword("nope"))).code).toBe("password_incorrect")
    await api.verifyPassword("secret")
    const result = await api.deleteOriginals([id])
    expect(result.succeeded).toBe(1)
    expect((await rejection(api.getAssetDetail(id))).code).toBe("not_found")
    clock += 6 * 60_000 // 기본 면제 5분 경과
    expect((await rejection(api.moveOriginals([page.items[1].assetId], "D:\\Moved"))).code).toBe("auth_required")
  })

  it("T-ui-api-04 [Business rule] tag add/remove updates search by tag text and detail; blank name rejected", async () => {
    // Arrange
    const api = createMockApi({ count: 500 })
    const id = (await api.searchAssets(req())).items[0].assetId
    // Act
    await api.updateAssetTags({ assetIds: [id], add: ["ZzUniqueTag"], remove: [] })
    const hit = await api.searchAssets(req({ query: "zzuniquetag" }))
    const detail = await api.getAssetDetail(id)
    await api.updateAssetTags({ assetIds: [id], add: [], remove: detail.tags.map((t) => t.tagId) })
    const gone = await api.searchAssets(req({ query: "zzuniquetag" }))
    const blank = await rejection(api.updateAssetTags({ assetIds: [id], add: ["  "], remove: [] }))
    // Assert
    expect(hit.items.map((i) => i.assetId)).toEqual([id])
    expect(detail.tags.some((t) => t.name === "ZzUniqueTag")).toBe(true)
    expect(gone.total).toBe(0)
    expect(blank.code).toBe("invalid_input")
  })
})

// Categories: Business rule / Time (T-ui-api-05) — 백엔드 guard.rs 회귀(면제 0분이면 인증 직후 명령이 항상 auth_required)와 같은 규칙
describe("mock api protection with zero exemption", () => {
  it("T-ui-api-05 [Business rule] 0-minute exemption allows exactly one protected command within 60s after verify", async () => {
    // Arrange
    let clock = 5_000_000
    const api = createMockApi({ count: 500, password: "secret12", now: () => clock })
    await api.updateSettings({ authExemptMinutes: 0 })
    const page = await api.searchAssets(req({ filters: { ...DEFAULT_FILTERS, includeOffline: false } }))
    const [a, b, c] = page.items.map((i) => i.assetId)
    // Act + Assert: 인증 직후 1회 허용, 상태에는 면제 없음
    const status = await api.verifyPassword("secret12")
    expect(status.authValidUntil).toBeNull()
    expect((await api.deleteOriginals([a])).succeeded).toBe(1)
    // 두 번째 명령은 다시 인증해야 한다
    expect((await rejection(api.deleteOriginals([b]))).code).toBe("auth_required")
    // 60초가 지나면 단일 허용도 만료된다
    await api.verifyPassword("secret12")
    clock += 60_001
    expect((await rejection(api.deleteOriginals([c]))).code).toBe("auth_required")
  })

  it("T-ui-api-05b [Validation] password shorter than 8 characters is rejected like the backend", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    // Act + Assert
    expect((await rejection(api.setPassword("short7!"))).code).toBe("invalid_input")
    expect((await api.setPassword("long-enough")).passwordSet).toBe(true)
  })
})

// Categories: Business rule (T-ui-api-06) — 보안 리뷰 #1·#8 백엔드 규칙과 mock 일치
describe("mock api settings guards", () => {
  it("T-ui-api-06 [Business rule] raising the exemption needs auth (single-use counts), lowering does not", async () => {
    // Arrange: 면제 0분에서 시작
    let clock = 9_000_000
    const api = createMockApi({ count: 10, password: "secret12", now: () => clock })
    await api.updateSettings({ authExemptMinutes: 0 })
    // Act + Assert: 인증 없이 올리기 거부
    expect((await rejection(api.updateSettings({ authExemptMinutes: 5 }))).code).toBe("auth_required")
    // 0분의 1회 허용으로 올릴 수 있다 (그렇지 않으면 0분에서 영영 못 올린다)
    await api.verifyPassword("secret12")
    expect((await api.updateSettings({ authExemptMinutes: 5 })).authExemptMinutes).toBe(5)
    // 내리기는 인증 없이 허용
    clock += 10 * 60_000
    expect((await api.updateSettings({ authExemptMinutes: 1 })).authExemptMinutes).toBe(1)
  })

  it("T-ui-api-06b [Business rule] data dir always gets a MAMData child so it never mixes with original folders", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    // Act + Assert
    expect((await api.updateSettings({ dataDir: "D:\\Work\\" })).dataDir).toBe("D:\\Work\\MAMData")
    expect((await api.updateSettings({ dataDir: "E:\\Keep\\MAMData" })).dataDir).toBe("E:\\Keep\\MAMData")
  })
})

describe("mock api paging and index", () => {
  it("T-ui-api-05a [Happy] backward and at pages equal the same slice of forward paging, with positions", async () => {
    // Arrange
    const api = createMockApi({ count: 400 })
    const all = (await api.searchAssets(req({ limit: 200 }))).items.map((i) => i.assetId)
    const first = await api.searchAssets(req({ limit: 50 }))
    const second = await api.searchAssets(req({ limit: 50, cursor: first.nextCursor }))
    // Act
    const back = await api.searchAssets(req({ limit: 50, cursor: second.prevCursor, direction: "backward" }))
    const seek = await api.searchAssets(req({ limit: 50, at: 120 }))
    const beyond = await api.searchAssets(req({ limit: 50, at: 100_000 }))
    // Assert
    expect(first.prevCursor).toBeNull()
    expect([first.startOffset, second.startOffset]).toEqual([0, 50])
    expect(back.items.map((i) => i.assetId)).toEqual(first.items.map((i) => i.assetId))
    expect(seek.items.map((i) => i.assetId)).toEqual(all.slice(120, 170))
    expect(seek.startOffset).toBe(120)
    expect(beyond.items).toHaveLength(0)
    expect([beyond.startOffset, beyond.nextCursor, beyond.prevCursor]).toEqual([beyond.total, null, null])
  })

  it("T-ui-api-05b [Validation] cursor + at, backward without cursor, and bad at are invalid_input", async () => {
    // Arrange
    const api = createMockApi({ count: 100 })
    const cursor = (await api.searchAssets(req({ limit: 10 }))).nextCursor
    // Act + Assert
    for (const bad of [req({ cursor, at: 3 }), req({ direction: "backward" }), req({ at: -1 }), req({ at: 1.5 })]) {
      expect((await rejection(api.searchAssets(bad))).code).toBe("invalid_input")
    }
  })

  it("T-ui-api-05c [Business rule] search_index sums to the total and follows the sort order", async () => {
    // Arrange
    const api = createMockApi({ count: 600 })
    // Act
    const byDate = await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort: { field: "capturedAt", direction: "desc" } })
    const byName = await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort: { field: "name", direction: "asc" } })
    const bySize = await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort: { field: "size", direction: "asc" } })
    const page = await api.searchAssets(req({ sort: { field: "capturedAt", direction: "desc" } }))
    // Assert
    for (const idx of [byDate, byName, bySize]) expect(idx.sections.reduce((n, s) => n + s.count, 0)).toBe(idx.total)
    expect(byDate.kind).toBe("month")
    expect(byDate.sections.map((s) => s.key).filter((k) => k !== "none")).toEqual(
      [...byDate.sections.map((s) => s.key).filter((k) => k !== "none")].sort().reverse(),
    )
    expect(byDate.generation).toBe(page.generation)
    expect(byName.kind).toBe("initial")
    expect(bySize).toMatchObject({ kind: "none", sections: [{ key: "all", count: bySize.total }] })
  })
})

describe("mock api gallery paging settings", () => {
  it("T-ui-api-06a [Happy] defaults and valid values round-trip", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    // Act
    const before = await api.getSettings()
    const after = await api.updateSettings({ galleryPaging: "paged", galleryPageSize: 200, indexRail: "off" })
    // Assert
    expect([before.galleryPaging, before.galleryPageSize, before.indexRail]).toEqual(["infinite", 100, "always"])
    expect([after.galleryPaging, after.galleryPageSize, after.indexRail]).toEqual(["paged", 200, "off"])
  })

  it("T-ui-api-06b [Validation] out-of-range values are invalid_input and change nothing", async () => {
    // Arrange
    const api = createMockApi({ count: 10 })
    const bad = [{ galleryPaging: "x" }, { galleryPageSize: 75 }, { galleryPageSize: 500 }, { indexRail: "sometimes" }]
    // Act + Assert
    for (const patch of bad) {
      const e = await rejection(api.updateSettings(patch as never))
      expect(e.code).toBe("invalid_input")
    }
    const s = await api.getSettings()
    expect([s.galleryPaging, s.galleryPageSize, s.indexRail]).toEqual(["infinite", 100, "always"])
  })
})

describe("mock search: hidden managed folders", () => {
  const base = { query: "", sort: { field: "capturedAt", direction: "desc" } as const, limit: 200, cursor: null }
  it("T-ui-hide-11 [Business rule] hidden folder is excluded from search, kept for folder scope and collection scope", async () => {
    // Arrange
    const api = createMockApi({ count: 3000 })
    const all = (await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort: base.sort })).total
    const inFolder = (await api.searchIndex({ query: "", filters: { ...DEFAULT_FILTERS, folder: { folderId: 1, relPath: null, recursive: true } }, sort: base.sort })).total
    expect(inFolder).toBeGreaterThan(0)
    // Act
    await api.setFolderHidden(1, true)
    // Assert
    const after = (await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort: base.sort })).total
    const scoped = (await api.searchIndex({ query: "", filters: { ...DEFAULT_FILTERS, folder: { folderId: 1, relPath: null, recursive: true } }, sort: base.sort })).total
    expect(after).toBe(all - inFolder)
    expect(scoped).toBe(inFolder)
    await api.setFolderHidden(1, false)
    expect((await api.searchIndex({ query: "", filters: DEFAULT_FILTERS, sort: base.sort })).total).toBe(all)
  })
})
