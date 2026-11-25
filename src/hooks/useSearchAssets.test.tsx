// Categories: Happy (T-ui-search-request-02, 03), Boundary / regression (T-ui-search-request-01, 04)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { SEMANTIC_SORT, resetAllStores, useSearchStore } from "@/stores"
import { useGalleryWindow } from "./galleryWindow/useGalleryWindow"
import { useSearchRequest } from "./useSearchAssets"

vi.mock("sonner", () => ({ toast: { info: vi.fn(), error: vi.fn(), success: vi.fn() } }))

beforeEach(() => resetAllStores())
afterEach(() => vi.restoreAllMocks())

const setMode = (mode: "keyword" | "hybrid") => act(() => useSearchStore.getState().setMode(mode))

describe("useSearchRequest", () => {
  it("T-ui-search-request-01 [Boundary] with an empty query, toggling the mode keeps the same request object", () => {
    // Arrange
    const { result } = renderHook(() => useSearchRequest())
    const first = result.current

    // Act
    setMode("hybrid")
    const afterHybrid = result.current
    setMode("keyword")

    // Assert
    expect(afterHybrid).toBe(first)
    expect(result.current).toBe(first)
    expect(first).not.toHaveProperty("mode")
  })

  it("T-ui-search-request-02 [Happy] with a query, switching to hybrid yields a new request with mode and relevance sort", () => {
    // Arrange
    act(() => useSearchStore.getState().setQuery("busan"))
    const { result } = renderHook(() => useSearchRequest())
    const keyword = result.current

    // Act
    setMode("hybrid")

    // Assert
    expect(result.current).not.toBe(keyword)
    expect(result.current.mode).toBe("hybrid")
    expect(result.current.sort).toEqual(SEMANTIC_SORT)
    expect(result.current.query).toBe("busan")
  })

  it("T-ui-search-request-03 [Happy] with a query, switching back to keyword removes mode and restores the user sort", () => {
    // Arrange
    act(() => {
      useSearchStore.getState().setQuery("busan")
      useSearchStore.getState().setSort({ field: "name", direction: "asc" })
    })
    const { result } = renderHook(() => useSearchRequest())
    setMode("hybrid")

    // Act
    setMode("keyword")

    // Assert
    expect(result.current).not.toHaveProperty("mode")
    expect(result.current.sort).toEqual({ field: "name", direction: "asc" })
  })

  it("T-ui-search-request-04 [Boundary] regression: toggling the mode with an empty query does not reload the gallery", async () => {
    // Arrange
    const api = createMockApi({ count: 500 })
    const indexSpy = vi.spyOn(api, "searchIndex")
    const searchSpy = vi.spyOn(api, "searchAssets")
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        <ApiProvider api={api}>{children}</ApiProvider>
      </QueryClientProvider>
    )
    const { result } = renderHook(() => useGalleryWindow({ mode: "infinite", pageSize: 100 }), { wrapper })
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    const indexCalls = indexSpy.mock.calls.length
    const searchCalls = searchSpy.mock.calls.length

    // Act
    setMode("hybrid")
    setMode("keyword")
    await act(async () => {
      await new Promise((r) => setTimeout(r, 50))
    })

    // Assert: 캐시가 유지되고 추가 조회가 없다
    expect(indexSpy).toHaveBeenCalledTimes(indexCalls)
    expect(searchSpy).toHaveBeenCalledTimes(searchCalls)
    expect(result.current.getItem(0)).toBeDefined()
  })
})
