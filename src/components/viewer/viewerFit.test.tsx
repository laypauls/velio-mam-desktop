// Categories: Happy (T-ui-viewer-fit-01, 02) / Boundary (T-ui-viewer-fit-03 exact limits, 04 viewer min wins, 05 no layout, 06 fitViewerWidth, 11 no sibling, 13 panel fit limits) / Business rule (T-ui-viewer-fit-07 left = right, 14 detail width) / Regression (T-ui-viewer-fit-09 stored width untouched, 10 ResizeObserver, 12 panel/detail toggles re-measure)
import { act, render } from "@testing-library/react"
import { useRef } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { resetAllStores, useViewStore } from "@/stores"
import { VIEWER_MIN_WIDTH } from "@/stores/view"
import { stubResizeObserver } from "@/test/webgl"
import { useFittedViewerWidth } from "./useFittedViewerWidth"
import { DETAIL_CLOSED_WIDTH, DETAIL_OPEN_WIDTH, fitViewerWidth, GALLERY_MIN_WIDTH, panelFitsWithViewer, SECTION_PANEL_WIDTH, viewerMaxWidth } from "./viewerFit"

describe("viewerMaxWidth / fitViewerWidth (pure)", () => {
  it("T-ui-viewer-fit-01 [Happy] max = (viewer + gallery) - gallery min — the same for left and right placement", () => {
    expect(viewerMaxWidth(1000)).toBe(1000 - GALLERY_MIN_WIDTH)
  })

  it("T-ui-viewer-fit-02 [Happy] the TopBar no longer limits the viewer: nothing but the gallery minimum is subtracted", () => {
    expect(viewerMaxWidth(1400)).toBe(1400 - GALLERY_MIN_WIDTH)
    expect(GALLERY_MIN_WIDTH).toBe(288)
  })

  it("T-ui-viewer-fit-03 [Boundary] total exactly at the limit gives exactly the viewer minimum", () => {
    expect(viewerMaxWidth(VIEWER_MIN_WIDTH + GALLERY_MIN_WIDTH)).toBe(VIEWER_MIN_WIDTH)
  })

  it("T-ui-viewer-fit-04 [Boundary] a window too narrow for the gallery minimum keeps the viewer at 320 (gallery yields)", () => {
    expect(viewerMaxWidth(100)).toBe(VIEWER_MIN_WIDTH)
    expect(viewerMaxWidth(VIEWER_MIN_WIDTH + GALLERY_MIN_WIDTH - 1)).toBe(VIEWER_MIN_WIDTH)
  })

  it("T-ui-viewer-fit-05 [Boundary] zero, negative or NaN total (no layout) means unbounded", () => {
    expect(viewerMaxWidth(0)).toBe(Number.POSITIVE_INFINITY)
    expect(viewerMaxWidth(-5)).toBe(Number.POSITIVE_INFINITY)
    expect(viewerMaxWidth(Number.NaN)).toBe(Number.POSITIVE_INFINITY)
  })

  it("T-ui-viewer-fit-06 [Boundary] fitViewerWidth clamps to [min, max] and never below the viewer minimum", () => {
    expect(fitViewerWidth(900, 500)).toBe(500)
    expect(fitViewerWidth(400, 500)).toBe(400)
    expect(fitViewerWidth(100, 500)).toBe(VIEWER_MIN_WIDTH)
    expect(fitViewerWidth(900, 10)).toBe(VIEWER_MIN_WIDTH)
    expect(fitViewerWidth(900, Number.POSITIVE_INFINITY)).toBe(900)
  })
})

describe("panelFitsWithViewer (pure)", () => {
  const need = SECTION_PANEL_WIDTH + VIEWER_MIN_WIDTH + 6 + GALLERY_MIN_WIDTH

  it("T-ui-viewer-fit-13 [Boundary] the panel fits exactly at panel + viewer min + handle + gallery min + detail, and not one pixel less", () => {
    expect(panelFitsWithViewer(need + DETAIL_OPEN_WIDTH, true)).toBe(true)
    expect(panelFitsWithViewer(need + DETAIL_OPEN_WIDTH - 1, true)).toBe(false)
  })

  it("T-ui-viewer-fit-14 [Business rule] a closed detail panel only reserves its narrow strip, so the same width fits", () => {
    const width = need + DETAIL_CLOSED_WIDTH
    expect(panelFitsWithViewer(width, false)).toBe(true)
    expect(panelFitsWithViewer(width, true)).toBe(false)
  })

  it("T-ui-viewer-fit-15 [Boundary] no layout (width 0 / NaN) counts as fitting so nothing collapses in jsdom or before first measure", () => {
    expect(panelFitsWithViewer(0, true)).toBe(true)
    expect(panelFitsWithViewer(Number.NaN, true)).toBe(true)
  })

  it("T-ui-viewer-fit-16 [Regression] the documented 960px minimum window cannot fit the panel with the detail panel open (so the panel is auto-collapsed)", () => {
    // 960 - 52(rail) = 908 < 240 + 320 + 6 + 288 + 320
    expect(panelFitsWithViewer(960 - 52, true)).toBe(false)
  })
})

/** [main][aside] (오른쪽 배치) / [aside][main] (왼쪽 배치) — 합계는 변수로 스텁한다. 보기 영역 폭은 고정 600 */
const geo = { total: 1400 }

function rect(width: number): DOMRect {
  return { x: 0, y: 0, top: 0, left: 0, right: width, bottom: 10, width, height: 10, toJSON: () => ({}) } as DOMRect
}

function stubRects() {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    if (this.tagName === "ASIDE") return rect(600)
    if (this.tagName === "MAIN") return rect(geo.total - 600)
    return rect(0)
  })
}

function Pane({ side }: { side: "left" | "right" }) {
  const ref = useRef<HTMLElement>(null)
  const width = useFittedViewerWidth(ref)
  const pane = <aside ref={ref} aria-label="pane" style={{ flexBasis: width }} />
  return side === "right" ? (
    <div>
      <main />
      {pane}
    </div>
  ) : (
    <div>
      {pane}
      <main />
    </div>
  )
}

const basis = () => document.querySelector("aside")?.style.flexBasis

describe("useFittedViewerWidth", () => {
  beforeEach(() => {
    resetAllStores()
    geo.total = 1400
    stubRects()
  })
  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it.each<"left" | "right">(["left", "right"])(
    "T-ui-viewer-fit-09 [Regression] %s: narrowing the window clamps the rendered width but never the stored viewerWidth; widening restores it",
    (side) => {
      // Arrange: 저장 폭 800, 넓은 창(1400) — 최대 1112
      useViewStore.setState({ viewerWidth: 800 })
      render(<Pane side={side} />)
      expect(basis()).toBe("800px")
      // Act: 창이 960 으로 좁아짐 → 최대 672
      geo.total = 960
      act(() => window.dispatchEvent(new Event("resize")))
      // Assert
      expect(basis()).toBe("672px")
      expect(useViewStore.getState().viewerWidth).toBe(800)
      // Act: 더 좁아져도 보기 영역 최소 폭(320)
      geo.total = 400
      act(() => window.dispatchEvent(new Event("resize")))
      expect(basis()).toBe(`${VIEWER_MIN_WIDTH}px`)
      expect(useViewStore.getState().viewerWidth).toBe(800)
      // Act: 다시 넓어짐
      geo.total = 1400
      act(() => window.dispatchEvent(new Event("resize")))
      expect(basis()).toBe("800px")
      expect(useViewStore.getState().viewerWidth).toBe(800)
    }
  )

  it("T-ui-viewer-fit-10 [Regression] left placement: the sibling main (not a column wrapper) is observed and a ResizeObserver callback re-measures; the pane itself is not observed", () => {
    // Arrange
    const ro = stubResizeObserver()
    useViewStore.setState({ viewerWidth: 700 })
    geo.total = 954
    render(<Pane side="left" />)
    // Assert: 최대 = max(320, 954 - 288) = 666 → 700 이 666 으로 줄어든다
    expect(basis()).toBe("666px")
    expect(ro.observing(document.querySelector("main") as Element)).toBe(true)
    expect(ro.observing(document.querySelector("aside") as Element)).toBe(false) // 되먹임 방지
    // Act: 창이 넓어짐 → ResizeObserver 콜백
    geo.total = 1400
    act(() => ro.trigger(document.querySelector("main") as Element))
    // Assert: 저장 폭 700 그대로
    expect(basis()).toBe("700px")
    expect(useViewStore.getState().viewerWidth).toBe(700)
  })

  it("T-ui-viewer-fit-11 [Boundary] without a layout (jsdom, all rects 0) the stored width is used unchanged", () => {
    // Arrange
    vi.restoreAllMocks() // 모든 rect 가 0
    useViewStore.setState({ viewerWidth: 640 })
    // Act
    render(<Pane side="right" />)
    // Assert
    expect(basis()).toBe("640px")
  })

  it("T-ui-viewer-fit-12 [Regression] toggling the section panel or the detail panel re-measures even when no observer fires", () => {
    // Arrange: 갤러리 폭을 측정값으로 바꿔 두고 viewerWidth 가 넓은 상태
    useViewStore.setState({ viewerWidth: 800, sidebarOpen: true })
    render(<Pane side="left" />)
    expect(basis()).toBe("800px")
    // Act: 패널이 접히고 창 폭이 줄어든 상태에서 상태만 바뀜(관찰자 없음)
    geo.total = 960
    act(() => useViewStore.setState({ sidebarOpen: false }))
    // Assert
    expect(basis()).toBe("672px")
    // Act: 상세 패널 토글도 같다
    geo.total = 1200
    act(() => useViewStore.setState({ detailOpen: false }))
    expect(basis()).toBe("800px")
  })

  it("T-ui-viewer-fit-17 [Boundary] no viewer pane (filmstrip mode renders none): measuring is skipped and nothing throws", () => {
    // Arrange + Act
    function NoPane() {
      const ref = useRef<HTMLElement>(null)
      return <span>{useFittedViewerWidth(ref)}</span>
    }
    const { container } = render(<NoPane />)
    // Assert: 제한 없음 → 저장 폭
    expect(container.textContent).toBe(String(useViewStore.getState().viewerWidth))
  })
})
