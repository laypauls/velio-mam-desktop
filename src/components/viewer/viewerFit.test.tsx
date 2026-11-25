// Categories: Happy (T-ui-viewer-fit-01, 02) / Boundary (T-ui-viewer-fit-03 exact limits, 04 viewer min wins, 05 no layout, 06 fitViewerWidth) / Business rule (T-ui-viewer-fit-07 chrome, 08 left vs right) / Regression (T-ui-viewer-fit-09 stored width untouched, 10 ResizeObserver, 11 no sibling)
import { act, render } from "@testing-library/react"
import { useRef } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { resetAllStores, useViewStore } from "@/stores"
import { VIEWER_MIN_WIDTH } from "@/stores/view"
import { stubResizeObserver } from "@/test/webgl"
import { useFittedViewerWidth } from "./useFittedViewerWidth"
import { fitViewerWidth, GALLERY_MIN_WIDTH, TOPBAR_MIN_WIDTH, viewerMaxWidth } from "./viewerFit"

describe("viewerMaxWidth / fitViewerWidth (pure)", () => {
  it("T-ui-viewer-fit-01 [Happy] right: max = total - gallery min", () => {
    expect(viewerMaxWidth("right", 1000, 0)).toBe(1000 - GALLERY_MIN_WIDTH)
  })

  it("T-ui-viewer-fit-02 [Happy] left: max = total - max(TopBar min, gallery min + chrome)", () => {
    expect(viewerMaxWidth("left", 1400, 0)).toBe(1400 - TOPBAR_MIN_WIDTH)
    expect(viewerMaxWidth("left", 1400, 400)).toBe(1400 - (GALLERY_MIN_WIDTH + 400))
  })

  it("T-ui-viewer-fit-03 [Boundary] total exactly at the limit gives exactly the viewer minimum", () => {
    expect(viewerMaxWidth("right", VIEWER_MIN_WIDTH + GALLERY_MIN_WIDTH, 0)).toBe(VIEWER_MIN_WIDTH)
    expect(viewerMaxWidth("left", VIEWER_MIN_WIDTH + TOPBAR_MIN_WIDTH, 0)).toBe(VIEWER_MIN_WIDTH)
  })

  it("T-ui-viewer-fit-04 [Boundary] a window too narrow for the gallery minimum keeps the viewer at 320 (gallery yields)", () => {
    // 왼쪽 배치, 최소 창 960: 보기 영역 + 열 = 954, 사이드바 256 + 상세 320 열림 → 열 최소 864 > 634
    expect(viewerMaxWidth("left", 954, 576)).toBe(VIEWER_MIN_WIDTH)
    expect(viewerMaxWidth("right", 100, 0)).toBe(VIEWER_MIN_WIDTH)
  })

  it("T-ui-viewer-fit-05 [Boundary] zero or negative total (no layout) means unbounded", () => {
    expect(viewerMaxWidth("right", 0, 0)).toBe(Number.POSITIVE_INFINITY)
    expect(viewerMaxWidth("left", -5, 100)).toBe(Number.POSITIVE_INFINITY)
    expect(viewerMaxWidth("left", Number.NaN, 0)).toBe(Number.POSITIVE_INFINITY)
  })

  it("T-ui-viewer-fit-06 [Boundary] fitViewerWidth clamps to [min, max] and never below the viewer minimum", () => {
    expect(fitViewerWidth(900, 500)).toBe(500)
    expect(fitViewerWidth(400, 500)).toBe(400)
    expect(fitViewerWidth(100, 500)).toBe(VIEWER_MIN_WIDTH)
    expect(fitViewerWidth(900, 10)).toBe(VIEWER_MIN_WIDTH)
    expect(fitViewerWidth(900, Number.POSITIVE_INFINITY)).toBe(900)
  })

  it("T-ui-viewer-fit-07 [Business rule] open sidebar/detail shrink the left-placement maximum; closed ones do not", () => {
    const closed = viewerMaxWidth("left", 1200, 0)
    const open = viewerMaxWidth("left", 1200, 576)
    expect(open).toBeLessThan(closed)
    expect(open).toBe(1200 - (GALLERY_MIN_WIDTH + 576))
  })

  it("T-ui-viewer-fit-08 [Business rule] with sidebar and detail closed the left and right maximums are equal (TopBar minimum = gallery minimum), and an open sidebar makes left smaller", () => {
    expect(viewerMaxWidth("left", 1200, 0)).toBe(viewerMaxWidth("right", 1200, 0))
    expect(viewerMaxWidth("left", 1200, 300)).toBeLessThan(viewerMaxWidth("right", 1200, 0))
  })
})

/** [aside 보기 영역][열] (왼쪽 배치) / [main][aside] (오른쪽 배치) — 합계는 window 변수로 스텁한다 */
const geo = { total: 1400, nav: 0, detail: 0 }

function rect(width: number): DOMRect {
  return { x: 0, y: 0, top: 0, left: 0, right: width, bottom: 10, width, height: 10, toJSON: () => ({}) } as DOMRect
}

function stubRects() {
  // 보기 영역은 사이드 형제와 합쳐 total 을 이룬다 — 보기 영역 폭은 고정 600, 형제는 나머지
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    if (this.tagName === "ASIDE") return rect(600)
    if (this.tagName === "NAV") return rect(geo.nav)
    if (this.hasAttribute("data-detail")) return rect(geo.detail)
    if (this.tagName === "MAIN" || this.hasAttribute("data-viewer-sibling")) return rect(geo.total - 600)
    return rect(0)
  })
}

function Pane({ side }: { side: "left" | "right" }) {
  const ref = useRef<HTMLElement>(null)
  const width = useFittedViewerWidth(ref, side)
  const pane = <aside ref={ref} aria-label="pane" style={{ flexBasis: width }} />
  return side === "right" ? (
    <div>
      <main />
      {pane}
    </div>
  ) : (
    <div>
      {pane}
      <div data-viewer-sibling="">
        <div>
          <nav />
          <main />
          <section data-detail="" />
        </div>
      </div>
    </div>
  )
}

const basis = () => document.querySelector("aside")?.style.flexBasis

describe("useFittedViewerWidth", () => {
  beforeEach(() => {
    resetAllStores()
    geo.total = 1400
    geo.nav = 0
    geo.detail = 0
    stubRects()
  })
  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it("T-ui-viewer-fit-09 [Regression] narrowing the window clamps the rendered width but never the stored viewerWidth; widening restores it", () => {
    // Arrange: 저장 폭 800, 넓은 창(1400) — 오른쪽 배치 최대 1112
    useViewStore.setState({ viewerWidth: 800 })
    render(<Pane side="right" />)
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
  })

  it("T-ui-viewer-fit-10 [Regression] left placement at the 960 minimum window with sidebar + detail open renders the viewer at 320; ResizeObserver on the sibling re-measures", () => {
    // Arrange
    const ro = stubResizeObserver()
    useViewStore.setState({ viewerWidth: 700 })
    geo.total = 954
    geo.nav = 256
    geo.detail = 320
    render(<Pane side="left" />)
    // Assert: 최대 = max(320, 954 - max(340, 288+576)) = 320
    expect(basis()).toBe(`${VIEWER_MIN_WIDTH}px`)
    expect(ro.observing(document.querySelector("[data-viewer-sibling]") as Element)).toBe(true)
    expect(ro.observing(document.querySelector("aside") as Element)).toBe(false) // 되먹임 방지
    // Act: 상세가 닫히고 창이 넓어짐 → ResizeObserver 콜백
    geo.total = 1400
    geo.detail = 0
    act(() => ro.trigger(document.querySelector("[data-viewer-sibling]") as Element))
    // Assert: 1400 - max(340, 288+256) = 840 → 저장 폭 700 그대로
    expect(basis()).toBe("700px")
    expect(useViewStore.getState().viewerWidth).toBe(700)
  })

  it("T-ui-viewer-fit-11 [Boundary] without a sibling (or in jsdom with no layout) the stored width is used unchanged", () => {
    // Arrange
    vi.restoreAllMocks() // 모든 rect 가 0
    useViewStore.setState({ viewerWidth: 640 })
    // Act
    render(<Pane side="right" />)
    // Assert
    expect(basis()).toBe("640px")
  })
})

describe("collapsed TopBar minimum", () => {
  it("T-ui-viewer-fit-12 [Boundary] left: with sidebar and detail closed the right column can shrink to the gallery minimum 288 thanks to the more menu, far below the old 560", () => {
    // Arrange + Act
    const max = viewerMaxWidth("left", 1400, 0)
    // Assert: 열이 갤러리 최소(카드 1개 폭 288)까지 줄어든다 — 이전 560 보다 272, 340 보다 52 더 넓힐 수 있다
    expect(TOPBAR_MIN_WIDTH).toBeLessThan(560)
    expect(TOPBAR_MIN_WIDTH).toBe(288)
    expect(TOPBAR_MIN_WIDTH).toBe(GALLERY_MIN_WIDTH)
    expect(max).toBe(1400 - TOPBAR_MIN_WIDTH)
  })
})
