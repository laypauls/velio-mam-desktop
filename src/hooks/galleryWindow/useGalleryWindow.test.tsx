// Categories: Happy / Boundary / External (errors, index_not_ready, stale cursor) / Concurrency (generation) / Time (fast-scroll defer, injected timers)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { useSetFolderHidden } from "../useFolderAdmin"
import { useUpdateSettings } from "../useSettings"
import { resetAllStores, useSearchStore } from "@/stores"
import { useGalleryWindow, type UseGalleryWindowOptions, type WindowTimers } from "./useGalleryWindow"

const toastInfo = vi.hoisted(() => vi.fn())
vi.mock("sonner", () => ({ toast: { info: toastInfo, error: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  toastInfo.mockClear()
  resetAllStores()
})
afterEach(() => vi.restoreAllMocks())

/** 결정적 타이머: 등록만 하고 `flush` 로 직접 실행 */
function fakeTimers() {
  const pending = new Map<number, () => void>()
  let id = 0
  const timers: WindowTimers = {
    set: (fn) => {
      pending.set(++id, fn)
      return id
    },
    clear: (h) => void pending.delete(h as number),
  }
  return { timers, size: () => pending.size, flush: () => [...pending.values()].forEach((fn) => (pending.clear(), fn())) }
}

function setup(api: ReturnType<typeof createMockApi>, initial: UseGalleryWindowOptions) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>{children}</ApiProvider>
    </QueryClientProvider>
  )
  return renderHook((o: UseGalleryWindowOptions) => useGalleryWindow(o), { wrapper, initialProps: initial })
}

const inf = (extra: Partial<UseGalleryWindowOptions> = {}): UseGalleryWindowOptions => ({ mode: "infinite", pageSize: 100, ...extra })
const hasAt = (spy: { mock: { calls: unknown[][] } }, at: number) =>
  spy.mock.calls.some(([r]) => (r as { at?: number }).at === at)

describe("useGalleryWindow (infinite)", () => {
  it("T-ui-win-60 [Happy] loads the index and first blocks; total comes from the index; loads at most 12 blocks", async () => {
    const api = createMockApi({ count: 5000 })
    const spy = vi.spyOn(api, "searchAssets")
    const { result } = setup(api, inf())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    await waitFor(() => expect(result.current.getItem(100)).toBeDefined())
    expect(result.current.total).toBeGreaterThan(1000)
    expect(result.current.range).toEqual([0, result.current.total])
    expect(result.current.index?.sections.reduce((s, x) => s + x.count, 0)).toBe(result.current.total)
    expect(result.current.isLoading).toBe(false)
    expect(result.current.loadedCount).toBeLessThanOrEqual(1200)
    expect(spy.mock.calls[0][0]).toMatchObject({ limit: 100, cursor: null })
  })

  it("T-ui-win-61 [Happy] a far jump seeks with at = block*100, neighbors then use cursors (prev/next)", async () => {
    const api = createMockApi({ count: 5000 })
    const spy = vi.spyOn(api, "searchAssets")
    const { result } = setup(api, inf())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    spy.mockClear()

    act(() => result.current.requestRange(2550, 2560))
    await waitFor(() => expect(result.current.getItem(2450)).toBeDefined())
    await waitFor(() => expect(result.current.getItem(2650)).toBeDefined())

    expect(hasAt(spy, 2500)).toBe(true)
    expect(spy.mock.calls.some(([r]) => r.direction === "backward" && r.cursor !== null)).toBe(true)
    expect(result.current.getItem(2555)?.assetId).toBeDefined()
  })

  it("T-ui-win-62 [Time] fast requests are deferred until idle and fire once", async () => {
    const api = createMockApi({ count: 5000 })
    const spy = vi.spyOn(api, "searchAssets")
    const ft = fakeTimers()
    const { result } = setup(api, inf({ timers: ft.timers }))
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    spy.mockClear()

    act(() => {
      result.current.requestRange(1000, 1010, { fast: true })
      result.current.requestRange(2000, 2010, { fast: true })
      result.current.requestRange(3000, 3010, { fast: true })
    })
    expect(spy).not.toHaveBeenCalled()
    expect(ft.size()).toBe(1) // 마지막 호출만 남는다
    act(() => ft.flush())
    await waitFor(() => expect(result.current.getItem(3005)).toBeDefined())
    expect(hasAt(spy, 3000)).toBe(true)
    expect(hasAt(spy, 1000)).toBe(false)
    expect(hasAt(spy, 2000)).toBe(false)
  })

  it("T-ui-win-63 [Boundary] block cap holds over many jumps", async () => {
    const api = createMockApi({ count: 6000 })
    const { result } = setup(api, inf())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    for (const p of [5000, 300, 4200, 1500, 5800, 10, 3300, 2700, 900, 4900, 2100, 5400]) {
      act(() => result.current.requestRange(p, p + 20))
      await waitFor(() => expect(result.current.getItem(p)).toBeDefined())
      expect(result.current.loadedCount).toBeLessThanOrEqual(1200)
    }
  })

  it("T-ui-win-64 [Concurrency] a generation change drops cached blocks and re-reads the index", async () => {
    const generation = { value: 0 }
    const api = createMockApi({ count: 3000, searchGeneration: generation })
    const idx = vi.spyOn(api, "searchIndex")
    const { result } = setup(api, inf())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    expect(result.current.generation).toBe(0)

    generation.value += 1
    act(() => result.current.requestRange(1500, 1510))

    await waitFor(() => expect(result.current.generation).toBe(1))
    await waitFor(() => expect(result.current.getItem(1505)).toBeDefined())
    expect(idx).toHaveBeenCalledTimes(2)
    expect(result.current.isError).toBe(false)
  })

  it("T-ui-win-65 [Concurrency] a sort change resets to the top and reloads under the new index", async () => {
    const api = createMockApi({ count: 3000 })
    const idx = vi.spyOn(api, "searchIndex")
    const { result } = setup(api, inf())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    act(() => result.current.requestRange(1500, 1510))
    await waitFor(() => expect(result.current.getItem(1505)).toBeDefined())
    const before = result.current.getItem(0)?.assetId

    act(() => useSearchStore.getState().setSort({ field: "name", direction: "asc" }))

    await waitFor(() => expect(result.current.shownSortField).toBe("name"))
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    expect(idx).toHaveBeenCalledTimes(2)
    expect(result.current.getItem(0)?.assetId).not.toBe(before)
    expect(result.current.getItem(1505)).toBeUndefined() // 옛 정렬의 블록이 남지 않는다
  })

  it("T-ui-win-66 [External] a stale cursor (invalid_input) resets once with an info toast and recovers", async () => {
    const api = createMockApi({ count: 3000 })
    const real = api.searchAssets.bind(api)
    let rejected = false
    vi.spyOn(api, "searchAssets").mockImplementation(async (req) => {
      if (req.cursor !== null && !rejected) {
        rejected = true
        throw { code: "invalid_input", message: "stale" }
      }
      return real(req)
    })
    const idx = vi.spyOn(api, "searchIndex")
    const { result } = setup(api, inf())

    await waitFor(() => expect(toastInfo).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(result.current.getItem(100)).toBeDefined())
    expect(result.current.isError).toBe(false)
    expect(idx).toHaveBeenCalledTimes(2)
    expect(toastInfo).toHaveBeenCalledTimes(1)
  })

  it("T-ui-win-67 [External] a block failure surfaces isError without looping; retry() recovers", async () => {
    const api = createMockApi({ count: 3000 })
    const real = api.searchAssets.bind(api)
    let broken = true
    const spy = vi.spyOn(api, "searchAssets").mockImplementation(async (req) => {
      if (broken) throw { code: "internal", message: "boom" }
      return real(req)
    })
    const { result } = setup(api, inf())
    await waitFor(() => expect(result.current.isError).toBe(true))
    await act(async () => {
      await Promise.resolve()
    })
    expect(spy).toHaveBeenCalledTimes(1)
    expect(result.current.error).toEqual({ code: "internal", message: "boom" })
    expect(result.current.indexPreparing).toBe(false)

    broken = false
    act(() => result.current.retry())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    expect(result.current.isError).toBe(false)
  })

  it("T-ui-win-68 [External] index_not_ready on the index silently retries and then loads", async () => {
    const api = createMockApi({ count: 3000 })
    const real = api.searchIndex.bind(api)
    let first = true
    const spy = vi.spyOn(api, "searchIndex").mockImplementation(async (req) => {
      if (first) {
        first = false
        throw { code: "index_not_ready", message: "not ready" }
      }
      return real(req)
    })
    const { result } = setup(api, inf({ indexRetryMs: 10 }))
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    expect(spy).toHaveBeenCalledTimes(2) // 첫 호출 실패 → 조용한 재시도
    expect(result.current.indexPreparing).toBe(false)
    expect(result.current.isError).toBe(false)
  })

  it("T-ui-win-69 [External] index_not_ready on a block fetch retries via the injected timer", async () => {
    const api = createMockApi({ count: 3000 })
    const real = api.searchAssets.bind(api)
    let first = true
    const spy = vi.spyOn(api, "searchAssets").mockImplementation(async (req) => {
      if (first) {
        first = false
        throw { code: "index_not_ready", message: "not ready" }
      }
      return real(req)
    })
    const ft = fakeTimers()
    const { result } = setup(api, inf({ timers: ft.timers }))
    await waitFor(() => expect(ft.size()).toBe(1))
    expect(result.current.isError).toBe(false)
    act(() => ft.flush())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    expect(spy.mock.calls.length).toBeGreaterThanOrEqual(2)
  })

  it("T-ui-win-70 [Boundary] an empty result loads nothing and is not an error", async () => {
    const api = createMockApi({ count: 3000 })
    const spy = vi.spyOn(api, "searchAssets")
    useSearchStore.setState({ query: "zzzz-no-such-file-name-zzzz" })
    const { result } = setup(api, inf())
    await waitFor(() => expect(result.current.index).toBeDefined())
    expect(result.current.total).toBe(0)
    expect(result.current.isLoading).toBe(false)
    expect(result.current.range).toEqual([0, 0])
    expect(spy).not.toHaveBeenCalled()
  })
})

describe("useGalleryWindow (paged)", () => {
  it("T-ui-win-71 [Happy] loads the current page, prefetches the next page exactly once, range is the page slice", async () => {
    const api = createMockApi({ count: 3000 })
    const spy = vi.spyOn(api, "searchAssets")
    const { result, rerender } = setup(api, { mode: "paged", pageSize: 50, page: 0 })
    await waitFor(() => expect(result.current.getItem(50)).toBeDefined()) // 다음 쪽 미리 받기
    expect(result.current.range).toEqual([0, 50])
    expect(result.current.pageCount).toBe(Math.ceil(result.current.total / 50))
    expect(spy).toHaveBeenCalledTimes(2)
    expect(spy.mock.calls.every(([r]) => r.limit === 50)).toBe(true)

    // 다음 쪽으로: 이미 있으므로 새 미리 받기(2쪽) 1회만
    rerender({ mode: "paged", pageSize: 50, page: 1 })
    await waitFor(() => expect(result.current.getItem(100)).toBeDefined())
    expect(result.current.range).toEqual([50, 100])
    expect(spy).toHaveBeenCalledTimes(3)
    expect(result.current.loadedCount).toBeLessThanOrEqual(150)
  })

  it("T-ui-win-72 [Boundary] a page beyond the last page is clamped; the last page is partial", async () => {
    const api = createMockApi({ count: 300 })
    const { result } = setup(api, { mode: "paged", pageSize: 50, page: 999 })
    await waitFor(() => expect(result.current.index).toBeDefined())
    const { total, pageCount } = result.current
    expect(result.current.page).toBe(pageCount - 1)
    expect(result.current.range).toEqual([(pageCount - 1) * 50, total])
    await waitFor(() => expect(result.current.getItem(total - 1)).toBeDefined())
  })

  it("T-ui-win-73 [Concurrency] page jumps keep at most 3 pages in memory", async () => {
    const api = createMockApi({ count: 3000 })
    const { result, rerender } = setup(api, { mode: "paged", pageSize: 50, page: 0 })
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    for (const page of [20, 3, 40, 7, 55, 12]) {
      rerender({ mode: "paged", pageSize: 50, page })
      await waitFor(() => expect(result.current.getItem(page * 50)).toBeDefined())
      expect(result.current.loadedCount).toBeLessThanOrEqual(150)
    }
  })

  it("T-ui-win-74 [Happy] switching mode starts from an empty cache", async () => {
    const api = createMockApi({ count: 3000 })
    const { result, rerender } = setup(api, inf())
    await waitFor(() => expect(result.current.getItem(0)).toBeDefined())
    rerender({ mode: "paged", pageSize: 50, page: 2 })
    expect(result.current.getItem(0)).toBeUndefined()
    await waitFor(() => expect(result.current.getItem(100)).toBeDefined())
    expect(result.current.range).toEqual([100, 150])
  })
})

describe("useGalleryWindow (capturedAtFallback)", () => {
  it("T-ui-win-80 [Business rule] changing the captured-date fallback drops cached blocks and reloads under the new order", async () => {
    // Arrange: 촬영일 정렬 오름차순 — 설정이 바뀌면 없는 촬영일의 위치가 달라진다
    useSearchStore.getState().setSort({ field: "capturedAt", direction: "asc" })
    const api = createMockApi({ count: 3000 })
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        <ApiProvider api={api}>{children}</ApiProvider>
      </QueryClientProvider>
    )
    const { result } = renderHook(() => ({ win: useGalleryWindow(inf()), update: useUpdateSettings({ silent: true }) }), { wrapper })
    await waitFor(() => expect(result.current.win.getItem(0)).toBeDefined())
    const before = result.current.win.getItem(0)!.assetId
    const idx = vi.spyOn(api, "searchIndex")

    // Act
    await act(async () => {
      await result.current.update.mutateAsync({ capturedAtFallback: "none" })
    })

    // Assert: 색인을 다시 받고, 첫 항목이 새 순서(촬영일 있는 자산이 앞)로 바뀐다
    await waitFor(() => expect(idx).toHaveBeenCalled())
    await waitFor(() => expect(result.current.win.getItem(0)?.capturedAt ?? null).not.toBeNull())
    expect(result.current.win.getItem(0)!.assetId).not.toBe(before)
    expect(result.current.win.isError).toBe(false)
  })
})

describe("useGalleryWindow (hidden managed folders)", () => {
  it("T-ui-hide-10 [Regression] toggling a folder's hidden state clears the cache instead of reusing old cursors (no cursorReset toast)", async () => {
    // Arrange: 커서에 "세대" 꼬리표를 달아, 토글 뒤에 토글 전 세대의 커서가 오면 백엔드처럼 거부한다
    const api = createMockApi({ count: 3000 })
    let gen = 0
    const real = api.searchAssets.bind(api)
    vi.spyOn(api, "searchAssets").mockImplementation(async (req) => {
      const tag = (c: string | null) => (c ? `${c}|${gen}` : null)
      let cursor = req.cursor
      if (cursor !== null) {
        const [raw, g] = cursor.split("|")
        if (Number(g) !== gen) throw { code: "invalid_input", message: "stale cursor" }
        cursor = raw
      }
      const page = await real({ ...req, cursor })
      return { ...page, nextCursor: tag(page.nextCursor), prevCursor: tag(page.prevCursor) }
    })
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        <ApiProvider api={api}>{children}</ApiProvider>
      </QueryClientProvider>
    )
    const { result } = renderHook(() => ({ win: useGalleryWindow(inf()), hide: useSetFolderHidden() }), { wrapper })
    await waitFor(() => expect(result.current.win.getItem(0)).toBeDefined())
    act(() => result.current.win.requestRange(250, 260))
    await waitFor(() => expect(result.current.win.getItem(250)).toBeDefined())
    const totalBefore = result.current.win.total
    gen = 1

    // Act
    await act(async () => {
      await result.current.hide.mutateAsync({ folderId: 1, hidden: true })
    })

    // Assert
    await waitFor(() => expect(result.current.win.total).toBeLessThan(totalBefore))
    await waitFor(() => expect(result.current.win.getItem(0)).toBeDefined())
    expect(toastInfo).not.toHaveBeenCalled()
    expect(result.current.win.isError).toBe(false)
  })
})
