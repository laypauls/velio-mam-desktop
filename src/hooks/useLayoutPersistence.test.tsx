// Categories: Happy (T-ui-layout-01, 03) / Concurrency (T-ui-layout-02) / Time (T-ui-layout-03) / External (T-ui-layout-04) / Validation (T-ui-layout-05, 06) / Business rule (T-ui-layout-07)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { LayoutSettings, Settings } from "@/lib/ipc-contract"
import { resetAllStores, useViewStore } from "@/stores"
import { LAYOUT_SAVE_DELAY_MS, normalizeLayout, useLayoutPersistence } from "./useLayoutPersistence"

function Host() {
  useLayoutPersistence()
  return null
}

beforeEach(() => {
  vi.useFakeTimers()
  resetAllStores()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

const STORED: LayoutSettings = { detailOpen: false, sidebarOpen: false, viewerWidth: 700, detailTab: "ai", filmstripHeight: 140 }

/** getSettings/updateSettings 를 즉시 해결되는 스파이로 바꾸고 훅만 렌더한다. `load: false` 면 getSettings 를 직접 풀어 준다 */
async function setup(stored: unknown = STORED, opts: { load?: boolean; updateFails?: boolean } = {}) {
  const api = createMockApi({ count: 1 })
  const base = await api.getSettings()
  const settings = { ...base, layout: stored } as Settings
  let release: () => void = () => undefined
  const get = vi.spyOn(api, "getSettings").mockImplementation(() =>
    opts.load === false ? new Promise((r) => (release = () => r(settings))) : Promise.resolve(settings)
  )
  const update = vi.spyOn(api, "updateSettings").mockImplementation(() =>
    opts.updateFails ? Promise.reject({ code: "io", message: "disk" }) : Promise.resolve(settings)
  )
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <Host />
      </QueryClientProvider>
    </ApiProvider>
  )
  await act(async () => {
    if (opts.load === false) return
    await vi.advanceTimersByTimeAsync(0)
  })
  return { get, update, release: () => act(async () => { release(); await vi.advanceTimersByTimeAsync(0) }) }
}

const layoutOf = () => {
  const { detailOpen, sidebarOpen, viewerWidth, detailTab, filmstripHeight } = useViewStore.getState()
  return { detailOpen, sidebarOpen, viewerWidth, detailTab, filmstripHeight }
}

describe("useLayoutPersistence", () => {
  it("T-ui-layout-01 [Happy] stored layout hydrates the store once without triggering a save", async () => {
    // Arrange + Act
    const { update } = await setup()
    // Assert
    expect(layoutOf()).toEqual(STORED)
    expect(useViewStore.getState().layoutRev).toBe(0)
    await act(() => vi.advanceTimersByTimeAsync(LAYOUT_SAVE_DELAY_MS * 2))
    expect(update).not.toHaveBeenCalled()
  })

  it("T-ui-layout-02 [Concurrency] a change made before settings load wins over the stored value and is saved", async () => {
    // Arrange: 설정이 아직 로드되지 않음
    const { update, release } = await setup(STORED, { load: false })
    // Act: 사용자가 먼저 상세 패널을 닫고, 그 뒤 저장값 도착
    act(() => useViewStore.getState().setDetailOpen(false))
    act(() => useViewStore.getState().setDetailTab("similar"))
    await release()
    // Assert: 저장값(sidebarOpen=false, 700, ai)으로 덮이지 않는다
    expect(layoutOf()).toEqual({ detailOpen: false, sidebarOpen: true, viewerWidth: 520, detailTab: "similar", filmstripHeight: 104 })
    await act(() => vi.advanceTimersByTimeAsync(LAYOUT_SAVE_DELAY_MS))
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith({ layout: { detailOpen: false, sidebarOpen: true, viewerWidth: 520, detailTab: "similar", filmstripHeight: 104 } })
  })

  it("T-ui-layout-03 [Time] saves once, 300ms after the last change (width drag collapses into one write)", async () => {
    // Arrange
    const { update } = await setup()
    // Act: 드래그처럼 연속 변경
    for (const w of [600, 610, 620, 630]) {
      act(() => useViewStore.getState().setViewerWidth(w))
      await act(() => vi.advanceTimersByTimeAsync(100))
    }
    // Assert: 마지막 변경 뒤 299ms 까지는 저장 없음
    await act(() => vi.advanceTimersByTimeAsync(LAYOUT_SAVE_DELAY_MS - 101))
    expect(update).not.toHaveBeenCalled()
    await act(() => vi.advanceTimersByTimeAsync(1))
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith({ layout: { ...STORED, viewerWidth: 630 } })
  })

  it("T-ui-layout-04 [External] a failed save shows an error toast and keeps the local state", async () => {
    // Arrange
    const toastError = vi.spyOn(toast, "error").mockReturnValue("t")
    const { update } = await setup(STORED, { updateFails: true })
    // Act
    act(() => useViewStore.getState().toggleSidebar())
    await act(() => vi.advanceTimersByTimeAsync(LAYOUT_SAVE_DELAY_MS))
    // Assert
    expect(update).toHaveBeenCalledTimes(1)
    expect(toastError).toHaveBeenCalledWith("파일 입출력 중 오류가 발생했습니다.")
    expect(useViewStore.getState().sidebarOpen).toBe(true)
  })

  it("T-ui-layout-05 [Validation] broken stored values fall back field by field; width is clamped to 320", async () => {
    // Arrange + Act
    await setup({ detailOpen: "no", sidebarOpen: false, viewerWidth: 100, detailTab: "bogus", extra: 1 })
    // Assert
    expect(layoutOf()).toEqual({ detailOpen: true, sidebarOpen: false, viewerWidth: 320, detailTab: "info", filmstripHeight: 104 })
  })

  it("T-ui-layout-06 [Boundary] normalizeLayout handles non-objects, NaN and fractional widths", () => {
    const defaults = { detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104 }
    expect(normalizeLayout(null)).toEqual(defaults)
    expect(normalizeLayout("x")).toEqual(defaults)
    expect(normalizeLayout({ viewerWidth: Number.NaN })).toEqual(defaults)
    expect(normalizeLayout({ viewerWidth: 320 }).viewerWidth).toBe(320)
    expect(normalizeLayout({ viewerWidth: 319.6 }).viewerWidth).toBe(320)
    expect(normalizeLayout({ viewerWidth: 640.4 }).viewerWidth).toBe(640)
  })

  it("T-ui-filmstrip-layout-01 [Boundary] filmstripHeight is clamped to 72..160 on normalize, hydrate and set; NaN and non-numbers fall back to 104", async () => {
    // Arrange + Act + Assert: normalize
    expect(normalizeLayout({ filmstripHeight: 10 }).filmstripHeight).toBe(72)
    expect(normalizeLayout({ filmstripHeight: 999 }).filmstripHeight).toBe(160)
    expect(normalizeLayout({ filmstripHeight: 120.4 }).filmstripHeight).toBe(120)
    expect(normalizeLayout({ filmstripHeight: Number.NaN }).filmstripHeight).toBe(104)
    expect(normalizeLayout({ filmstripHeight: "140" }).filmstripHeight).toBe(104)
    // setter
    act(() => useViewStore.getState().setFilmstripHeight(5000))
    expect(useViewStore.getState().filmstripHeight).toBe(160)
    act(() => useViewStore.getState().setFilmstripHeight(1))
    expect(useViewStore.getState().filmstripHeight).toBe(72)
  })

  it("T-ui-filmstrip-layout-02 [Happy] stored filmstripHeight hydrates without saving, and a drag change is saved with the other three fields", async () => {
    // Arrange
    const { update } = await setup()
    expect(useViewStore.getState().filmstripHeight).toBe(140)
    expect(useViewStore.getState().layoutRev).toBe(0)
    // Act
    act(() => useViewStore.getState().setFilmstripHeight(90))
    await act(() => vi.advanceTimersByTimeAsync(LAYOUT_SAVE_DELAY_MS))
    // Assert
    expect(update).toHaveBeenCalledWith({ layout: { ...STORED, filmstripHeight: 90 } })
  })

  it("T-ui-layout-07 [Business rule] setting a value equal to the current one does not schedule a save", async () => {
    // Arrange
    const { update } = await setup()
    // Act
    act(() => {
      useViewStore.getState().setDetailOpen(false)
      useViewStore.getState().setDetailTab("ai")
      useViewStore.getState().setViewerWidth(700)
    })
    await act(() => vi.advanceTimersByTimeAsync(LAYOUT_SAVE_DELAY_MS * 2))
    // Assert
    expect(update).not.toHaveBeenCalled()
  })
})
