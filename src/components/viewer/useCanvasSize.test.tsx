// T-ui-mpv-size-01..: 캔버스 크기 측정(contain)·렌더 크기 보고(디바운스)
// Categories: Happy (-03 measure, -04 debounced report) / Boundary (-01 fit math, -02 device px clamp, -06 collapsed stage) / Business rule (-05 gate, -07 aspect) / External failure (-08 failing command)
import { act, render } from "@testing-library/react"
import { useState } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { CanvasSize } from "@/lib/ipc-contract-player"
import { stubResizeObserver } from "@/test/webgl"
import { containFit, RENDER_SIZE_DEBOUNCE_MS, toDevicePx, useCanvasSize, useRenderSizeReport, type CanvasMetrics } from "./useCanvasSize"

let rect = { width: 1000, height: 600 }

beforeEach(() => {
  rect = { width: 1000, height: 600 }
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(
    () => ({ x: 0, y: 0, left: 0, top: 0, right: rect.width, bottom: rect.height, width: rect.width, height: rect.height, toJSON: () => ({}) }) as DOMRect
  )
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

interface Props {
  aspect?: number
  live?: boolean
  baseline?: CanvasSize | null
}

/** 무대 요소 하나에 두 훅을 건다 — 측정 결과를 `seen` 으로 노출한다 */
function mount(initial: Props = {}) {
  const ro = stubResizeObserver()
  const api = createMockApi({ count: 5 })
  const setSize = vi.spyOn(api, "playerSetRenderSize").mockResolvedValue(undefined)
  const seen: { metrics: CanvasMetrics } = { metrics: { size: null, visible: false } }
  function Harness({ aspect = 16 / 9, live = true, baseline = null }: Props) {
    const [el, setEl] = useState<HTMLDivElement | null>(null)
    const metrics = useCanvasSize(el, aspect)
    useRenderSizeReport(metrics, live, baseline)
    seen.metrics = metrics
    return <div ref={setEl} data-testid="stage" />
  }
  const tree = (p: Props) => (
    <ApiProvider api={api}>
      <Harness {...p} />
    </ApiProvider>
  )
  const view = render(tree(initial))
  return {
    api, setSize, seen, ro,
    rerender: (p: Props) => view.rerender(tree(p)),
    stage: () => view.getByTestId("stage"),
  }
}

describe("containFit / toDevicePx", () => {
  it("T-ui-mpv-size-01 [Boundary] containFit keeps the aspect inside the stage (wide stage → height limited, tall stage → width limited) and yields 0 for empty stages or bad aspects", () => {
    // Arrange + Act + Assert
    expect(containFit(1000, 600, 16 / 9)).toEqual({ width: 1000, height: 562.5 })
    expect(containFit(1600, 600, 16 / 9)).toEqual({ width: 600 * (16 / 9), height: 600 })
    expect(containFit(400, 900, 1)).toEqual({ width: 400, height: 400 })
    expect(containFit(0, 600, 16 / 9)).toEqual({ width: 0, height: 0 })
    expect(containFit(1000, 600, 0)).toEqual({ width: 0, height: 0 })
    expect(containFit(1000, 600, Number.NaN)).toEqual({ width: 0, height: 0 })
  })

  it("T-ui-mpv-size-02 [Boundary] device px = CSS px × devicePixelRatio, at least 2×2 and at most 16384 per side", () => {
    // Arrange + Act + Assert
    expect(toDevicePx({ width: 1000, height: 562.5 }, 1.5)).toEqual({ width: 1500, height: 844 })
    expect(toDevicePx({ width: 0.2, height: 1 }, 1)).toEqual({ width: 2, height: 2 })
    expect(toDevicePx({ width: 99999, height: 20000 }, 2)).toEqual({ width: 16384, height: 16384 })
  })
})

describe("useCanvasSize", () => {
  it("T-ui-mpv-size-03 [Happy] measures the contained size in device px at mount (aspect from the asset) and honours devicePixelRatio", async () => {
    // Arrange
    vi.stubGlobal("devicePixelRatio", 1.25)
    // Act
    const t = mount({ aspect: 16 / 9 })
    // Assert: 1000×562.5 CSS → 1250×703 장치 px
    expect(t.seen.metrics).toEqual({ size: { width: 1250, height: 703 }, visible: true })
  })

  it("T-ui-mpv-size-04 [Happy] a size change while live is reported once after the 120 ms debounce; changes inside the window coalesce; the baseline is not re-sent", () => {
    // Arrange
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    const baseline = { width: 1000, height: 563 }
    const t = mount({ baseline })
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS * 2))
    expect(t.setSize).not.toHaveBeenCalled() // 열기에 실어 보낸 크기와 같다
    // Act: 연속 변경 (무대 폭 줄임)
    rect = { width: 800, height: 600 }
    act(() => t.ro.trigger(t.stage()))
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS - 1))
    rect = { width: 640, height: 600 }
    act(() => t.ro.trigger(t.stage()))
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS - 1))
    expect(t.setSize).not.toHaveBeenCalled()
    act(() => vi.advanceTimersByTime(1))
    // Assert: 마지막 크기 하나만
    expect(t.setSize).toHaveBeenCalledTimes(1)
    expect(t.setSize).toHaveBeenCalledWith(640, 360)
    // 같은 크기가 다시 측정돼도 보내지 않는다
    act(() => t.ro.trigger(t.stage()))
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS * 2))
    expect(t.setSize).toHaveBeenCalledTimes(1)
  })

  it("T-ui-mpv-size-05 [Business rule] nothing is reported while the session is not live; going live compares against the baseline", () => {
    // Arrange
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    const t = mount({ live: false })
    rect = { width: 800, height: 600 }
    act(() => t.ro.trigger(t.stage()))
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS * 2))
    // Assert: 열리기 전에는 보내지 않는다
    expect(t.setSize).not.toHaveBeenCalled()
    // Act: 열렸다 — 열기에 실은 크기(1000×563)와 지금(800×450)이 다르다
    t.rerender({ live: true, baseline: { width: 1000, height: 563 } })
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS))
    // Assert
    expect(t.setSize).toHaveBeenCalledWith(800, 450)
  })

  it("T-ui-mpv-size-06 [Boundary] a collapsed (0×0) stage keeps the last size, reports invisible and sends nothing; restoring the size recovers", () => {
    // Arrange
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    const t = mount({ baseline: { width: 1000, height: 563 } })
    // Act: 보기 영역이 접힘
    rect = { width: 0, height: 0 }
    act(() => t.ro.trigger(t.stage()))
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS * 2))
    // Assert
    expect(t.seen.metrics).toEqual({ size: { width: 1000, height: 563 }, visible: false })
    expect(t.setSize).not.toHaveBeenCalled()
    // Act: 같은 크기로 복원 — 보낼 것 없음
    rect = { width: 1000, height: 600 }
    act(() => t.ro.trigger(t.stage()))
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS * 2))
    expect(t.seen.metrics.visible).toBe(true)
    expect(t.setSize).not.toHaveBeenCalled()
  })

  it("T-ui-mpv-size-07 [Business rule] a different video aspect (first frame / asset) re-measures the contained size", () => {
    // Arrange
    const t = mount({ aspect: 16 / 9 })
    expect(t.seen.metrics.size).toEqual({ width: 1000, height: 563 })
    // Act: 세로 영상 (9:16) → 높이에 맞춘다
    t.rerender({ aspect: 9 / 16 })
    // Assert
    expect(t.seen.metrics.size).toEqual({ width: 338, height: 600 })
  })

  it("T-ui-mpv-size-08 [External failure] a failing player_set_render_size is swallowed and the next change is sent again", () => {
    // Arrange
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    const t = mount({ baseline: { width: 1000, height: 563 } })
    t.setSize.mockRejectedValue({ code: "internal", message: "x" })
    // Act
    rect = { width: 800, height: 600 }
    act(() => t.ro.trigger(t.stage()))
    expect(() => act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS))).not.toThrow()
    t.setSize.mockResolvedValue(undefined)
    rect = { width: 700, height: 600 }
    act(() => t.ro.trigger(t.stage()))
    act(() => vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS))
    // Assert
    expect(t.setSize).toHaveBeenCalledTimes(2)
    expect(t.setSize).toHaveBeenLastCalledWith(700, 394)
  })

  it("T-ui-mpv-size-09 [Happy] a window resize (also fired when devicePixelRatio changes) re-measures without the observer", () => {
    // Arrange
    const t = mount()
    expect(t.seen.metrics.size).toEqual({ width: 1000, height: 563 })
    // Act
    vi.stubGlobal("devicePixelRatio", 2)
    act(() => void window.dispatchEvent(new Event("resize")))
    // Assert
    expect(t.seen.metrics.size).toEqual({ width: 2000, height: 1125 })
  })
})
