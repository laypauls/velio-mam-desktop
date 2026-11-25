// Categories: Happy (T-ui-sort-01, 04) / Business rule (T-ui-sort-02, 03) / Boundary (T-ui-sort-03)
import { beforeEach, describe, expect, it } from "vitest"
import { DEFAULT_FILTERS, DEFAULT_SORT, canReorder, requestSort, resetAllStores, selectCanReorder, useSearchStore, useViewStore } from "@/stores"

const search = () => useSearchStore.getState()

beforeEach(() => resetAllStores())

describe("collection order sort", () => {
  it("T-ui-sort-01 [Happy] entering a collection view sets collectionOrder asc and remembers the previous sort", () => {
    // Arrange
    search().setSort({ field: "name", direction: "desc" })
    // Act
    search().setView({ kind: "collection", collectionId: 3 })
    // Assert
    expect(search().sort).toEqual({ field: "collectionOrder", direction: "asc" })
    expect(search().sortBeforeCollection).toEqual({ field: "name", direction: "desc" })
  })

  it("T-ui-sort-02 [Business rule] leaving the collection restores the previous sort; moving between collections keeps the memory", () => {
    // Arrange
    search().setSort({ field: "size", direction: "asc" })
    search().setView({ kind: "collection", collectionId: 3 })
    // Act 1: 다른 컬렉션으로 — 정렬은 다시 컬렉션 순서, 기억은 유지
    search().setView({ kind: "collection", collectionId: 4 })
    expect(search().sort.field).toBe("collectionOrder")
    // Act 2: 라이브러리로
    search().setView({ kind: "all" })
    // Assert
    expect(search().sort).toEqual({ field: "size", direction: "asc" })
    expect(search().sortBeforeCollection).toBeNull()
  })

  it("T-ui-sort-02b [Business rule] a sort the user changed inside the collection is kept when leaving", () => {
    // Arrange
    search().setView({ kind: "collection", collectionId: 3 })
    search().setSort({ field: "name", direction: "asc" })
    // Act
    search().setView({ kind: "all" })
    // Assert
    expect(search().sort).toEqual({ field: "name", direction: "asc" })
  })

  it("T-ui-sort-02c [Boundary] leaving with no remembered sort falls back to the default", () => {
    // Arrange: 저장한 검색 등으로 컬렉션 순서가 직접 들어온 상태
    useSearchStore.setState({ sort: { field: "collectionOrder", direction: "asc" }, sortBeforeCollection: null })
    // Act
    search().setView({ kind: "image" })
    // Assert
    expect(search().sort).toEqual(DEFAULT_SORT)
  })

  it("T-ui-sort-03 [Validation] requestSort drops collectionOrder without an effective collection but keeps it with one", () => {
    // Arrange
    const co = { field: "collectionOrder", direction: "asc" } as const
    // Act + Assert
    expect(requestSort({ kind: "all" }, DEFAULT_FILTERS, co)).toEqual(DEFAULT_SORT)
    expect(requestSort({ kind: "collection", collectionId: 2 }, DEFAULT_FILTERS, co)).toEqual(co)
    // 필터가 컬렉션을 담고 있으면 컬렉션 보기 밖에서도 유효
    expect(requestSort({ kind: "all" },{ ...DEFAULT_FILTERS, collectionId: 2 }, co)).toEqual(co)
    // 다른 정렬은 그대로
    expect(requestSort({ kind: "all" }, DEFAULT_FILTERS, { field: "name", direction: "desc" })).toEqual({ field: "name", direction: "desc" })
  })

  it("T-ui-sort-04 [Business rule] canReorder needs a collection view with ascending collection order, and is off for semantic search", () => {
    // Arrange
    const view = { kind: "collection", collectionId: 2 } as const
    // Act + Assert
    expect(canReorder(view, { field: "collectionOrder", direction: "asc" })).toBe(true)
    expect(canReorder(view, { field: "collectionOrder", direction: "desc" })).toBe(false)
    expect(canReorder(view, { field: "name", direction: "asc" })).toBe(false)
    expect(canReorder({ kind: "all" }, { field: "collectionOrder", direction: "asc" })).toBe(false)
    // 의미 검색(관련도 정렬)이 켜지면 컬렉션 보기여도 꺼진다
    search().setView(view)
    expect(selectCanReorder(search())).toBe(true)
    search().setMode("hybrid")
    search().setQuery("바다")
    expect(selectCanReorder(search())).toBe(false)
  })

  it("T-ui-sbar-05 [Happy] the sidebar tab defaults to library, is kept in memory, and resets with the stores", () => {
    // Arrange + Act
    expect(useViewStore.getState().sidebarTab).toBe("library")
    useViewStore.getState().setSidebarTab("collections")
    // Assert
    expect(useViewStore.getState().sidebarTab).toBe("collections")
    resetAllStores()
    expect(useViewStore.getState().sidebarTab).toBe("library")
  })
})
