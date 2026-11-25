// Categories: Happy (T-ui-play-24, 27) / Business rule (T-ui-play-25, 26) / Boundary (T-ui-play-28)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, renderHook, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { PlayQueueItem } from "@/lib/ipc-contract"
import { resetAllStores, usePlaybackStore } from "@/stores"
import { PlaybackBar } from "./PlaybackBar"
import { useSlideshowTimer } from "./useSlideshowTimer"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  resetAllStores()
  vi.useFakeTimers()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

const advance = (ms: number) => act(() => void vi.advanceTimersByTime(ms))

describe("useSlideshowTimer", () => {
  it("T-ui-play-24 [Happy] fires once after the configured seconds", () => {
    // Arrange
    const onElapsed = vi.fn()
    renderHook(() => useSlideshowTimer(true, 5, 1, onElapsed))
    // Act + Assert
    advance(4999)
    expect(onElapsed).not.toHaveBeenCalled()
    advance(1)
    expect(onElapsed).toHaveBeenCalledTimes(1)
  })

  it("T-ui-play-25 [Business rule] pausing keeps the remaining time and resuming continues from it", () => {
    // Arrange
    const onElapsed = vi.fn()
    const { rerender } = renderHook(({ active }) => useSlideshowTimer(active, 5, 1, onElapsed), { initialProps: { active: true } })
    // Act: 3초 보여 주고 일시정지, 한참 지나도 안 넘어간다
    advance(3000)
    rerender({ active: false })
    advance(60_000)
    expect(onElapsed).not.toHaveBeenCalled()
    // 재개 — 남은 2초
    rerender({ active: true })
    advance(1999)
    expect(onElapsed).not.toHaveBeenCalled()
    advance(1)
    // Assert
    expect(onElapsed).toHaveBeenCalledTimes(1)
  })

  it("T-ui-play-26 [Business rule] a new resetKey (next item) starts the full duration again", () => {
    // Arrange
    const onElapsed = vi.fn()
    const { rerender } = renderHook(({ key }) => useSlideshowTimer(true, 5, key, onElapsed), { initialProps: { key: 1 } })
    advance(4000)
    // Act
    rerender({ key: 2 })
    advance(4999)
    // Assert
    expect(onElapsed).not.toHaveBeenCalled()
    advance(1)
    expect(onElapsed).toHaveBeenCalledTimes(1)
  })

  it("T-ui-play-27 [Happy] progress rises with elapsed time and resets for the next item", () => {
    // Arrange
    const { result, rerender } = renderHook(({ key }) => useSlideshowTimer(true, 10, key, () => undefined), { initialProps: { key: 1 } })
    // Act + Assert
    advance(5000)
    expect(result.current).toBeGreaterThan(0.4)
    expect(result.current).toBeLessThanOrEqual(0.6)
    rerender({ key: 2 })
    expect(result.current).toBe(0)
  })

  it("T-ui-play-28 [Boundary] inactive from the start never fires", () => {
    // Arrange
    const onElapsed = vi.fn()
    renderHook(() => useSlideshowTimer(false, 1, 1, onElapsed))
    // Act
    advance(10_000)
    // Assert
    expect(onElapsed).not.toHaveBeenCalled()
  })
})

const img = (assetId: number): PlayQueueItem => ({ assetId, mediaType: "image", availability: "online", fileName: `p${assetId}.jpg`, title: null, durationMs: null })

describe("PlaybackBar with images", () => {
  function mountBar() {
    const api = createMockApi({ count: 5 })
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    return render(
      <ApiProvider api={api}>
        <QueryClientProvider client={queryClient}>
          <PlaybackBar />
        </QueryClientProvider>
      </ApiProvider>
    )
  }

  it("T-ui-play-29 [Happy] an image advances to the next after the default 5 s; the pause button holds it", () => {
    // Arrange
    act(() => usePlaybackStore.getState().start(1, [img(1), img(2), img(3)]))
    mountBar()
    // Act + Assert: 5초 → 다음
    advance(5000)
    expect(usePlaybackStore.getState().currentId).toBe(2)
    // 일시정지 → 오래 지나도 유지
    act(() => screen.getByRole("button", { name: "일시정지" }).click())
    expect(usePlaybackStore.getState().status).toBe("paused")
    advance(60_000)
    expect(usePlaybackStore.getState().currentId).toBe(2)
    // 재개 → 다시 흐른다
    act(() => screen.getByRole("button", { name: "재개" }).click())
    advance(5000)
    expect(usePlaybackStore.getState().currentId).toBe(3)
  })

  it("T-ui-play-30 [Business rule] the last image with repeat off stops and the pause button disappears", () => {
    // Arrange
    act(() => usePlaybackStore.getState().start(1, [img(1)]))
    mountBar()
    // Act
    advance(5000)
    // Assert
    expect(usePlaybackStore.getState().status).toBe("stopped")
    expect(screen.queryByRole("button", { name: "일시정지" })).not.toBeInTheDocument()
  })
})

describe("Space on playback items", () => {
  const video = (assetId: number): PlayQueueItem => ({ ...img(assetId), mediaType: "video", fileName: `v${assetId}.mp4` })
  const press = (target: EventTarget = document.body) =>
    act(() => void target.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true, cancelable: true })))

  function mountBar() {
    const api = createMockApi({ count: 5 })
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    return render(
      <ApiProvider api={api}>
        <QueryClientProvider client={queryClient}>
          <input aria-label="입력" />
          <PlaybackBar />
        </QueryClientProvider>
      </ApiProvider>
    )
  }

  it("T-ui-play-31 [Happy] Space on an image pauses then resumes the slideshow keeping the remaining time; the bar reflects it", () => {
    // Arrange
    act(() => usePlaybackStore.getState().start(1, [img(1), img(2)]))
    mountBar()
    advance(3000)
    // Act: 일시정지
    press()
    expect(usePlaybackStore.getState().status).toBe("paused")
    expect(screen.getByRole("button", { name: "재개" })).toBeInTheDocument()
    advance(60_000)
    expect(usePlaybackStore.getState().currentId).toBe(1)
    // 재개 — 남은 2초
    press()
    expect(screen.getByRole("button", { name: "일시정지" })).toBeInTheDocument()
    advance(1999)
    expect(usePlaybackStore.getState().currentId).toBe(1)
    advance(1)
    // Assert
    expect(usePlaybackStore.getState().currentId).toBe(2)
  })

  it("T-ui-play-32 [Validation] Space inside an input is ignored", () => {
    // Arrange
    act(() => usePlaybackStore.getState().start(1, [img(1), img(2)]))
    mountBar()
    // Act
    press(screen.getByRole("textbox", { name: "입력" }))
    // Assert
    expect(usePlaybackStore.getState().status).toBe("playing")
  })

  it("T-ui-play-33 [Business rule] Space on a video item is left to the player keys (no pause toggle from the playback hook)", () => {
    // Arrange
    act(() => usePlaybackStore.getState().start(1, [video(1), img(2)]))
    mountBar()
    // Act
    press()
    // Assert
    expect(usePlaybackStore.getState().status).toBe("playing")
  })
})
