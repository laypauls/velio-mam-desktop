// Categories: Happy (T-ui-vresize-01, 02, 05) / Boundary (T-ui-vresize-03 gallery min clamp, 04 viewer min clamp, 06 no sibling) / Business rule (T-ui-vresize-07) / Regression (T-ui-vresize-08)
import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { resetAllStores, useViewStore } from "@/stores"
import { VIEWER_MIN_WIDTH } from "@/stores/view"
import { ViewerResizeHandle } from "./ViewerResizeHandle"
import { useRef } from "react"

/** 레이아웃 스텁 값 — 한 줄: [viewer 500] [사이드바 256] [main 600] (왼쪽 배치) / [main 600] [viewer 500] (오른쪽 배치) */
interface Geometry {
  paneLeft: number
  paneWidth: number
  /** 오른쪽 배치: `main` 폭 / 왼쪽 배치: 오른쪽 열(`data-viewer-sibling`) 폭 */
  galleryWidth: number
  /** 왼쪽 배치의 사이드바(nav) 폭 — 오른쪽 열 안에 든다 */
  navWidth?: number
}

function rect(left: number, width: number): DOMRect {
  return { x: left, y: 0, top: 0, left, right: left + width, bottom: 100, width, height: 100, toJSON: () => ({}) } as DOMRect
}

function stubGeometry(g: Geometry) {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    if (this.tagName === "ASIDE") return rect(g.paneLeft, g.paneWidth)
    if (this.tagName === "NAV") return rect(0, g.navWidth ?? 0)
    if (this.tagName === "MAIN" || this.hasAttribute("data-viewer-sibling")) return rect(0, g.galleryWidth)
    return rect(0, 0)
  })
}

/** withMain=false 면 형제 영역(오른쪽 배치의 main / 왼쪽 배치의 data-viewer-sibling 열)을 만들지 않는다 */
function Harness({ side, withMain = true }: { side: "left" | "right"; withMain?: boolean }) {
  const ref = useRef<HTMLElement>(null)
  const handle = <ViewerResizeHandle paneRef={ref} side={side} />
  const pane = <aside ref={ref} />
  return (
    <div>
      {withMain && side === "right" && <main />}
      {side === "right" ? (
        <>
          {handle}
          {pane}
        </>
      ) : (
        <>
          {pane}
          {handle}
        </>
      )}
      {withMain && side === "left" && (
        <div data-viewer-sibling="">
          <header />
          <div>
            <nav />
            <main />
          </div>
        </div>
      )}
    </div>
  )
}

beforeEach(() => {
  resetAllStores()
  useViewStore.setState({ viewerWidth: 500 })
})
afterEach(() => {
  vi.restoreAllMocks()
})

const handle = () => screen.getByRole("separator", { name: "보기 영역 폭 조절" })
const width = () => useViewStore.getState().viewerWidth

describe("ViewerResizeHandle — direction", () => {
  it("T-ui-vresize-01 [Happy] right side: dragging uses pane.right - clientX and ArrowLeft widens", () => {
    // Arrange: 보기 영역 x 700..1200, 갤러리 700
    stubGeometry({ paneLeft: 700, paneWidth: 500, galleryWidth: 700 })
    render(<Harness side="right" />)
    // Act: 분할선을 x=800 으로 끌기 → 1200 - 800
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 800, pointerId: 1 })
    // Assert
    expect(width()).toBe(400)
    // Act: ← 로 넓힘, → 로 좁힘
    fireEvent.keyDown(handle(), { key: "ArrowLeft" })
    expect(width()).toBe(424)
    fireEvent.keyDown(handle(), { key: "ArrowRight" })
    expect(width()).toBe(400)
  })

  it("T-ui-vresize-02 [Happy] left side: dragging uses clientX - pane.left and ArrowRight widens", () => {
    // Arrange: 보기 영역 x 0..500, 오른쪽 열 900 (최대 900+500-288 = 1112)
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 900 })
    render(<Harness side="left" />)
    // Act: 분할선을 x=620 으로 끌기 → 620 - 0
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 620, pointerId: 1 })
    // Assert
    expect(width()).toBe(620)
    // Act
    fireEvent.keyDown(handle(), { key: "ArrowRight" })
    expect(width()).toBe(644)
    fireEvent.keyDown(handle(), { key: "ArrowLeft" })
    expect(width()).toBe(620)
  })

  it("T-ui-vresize-05 [Validation] pointer moves without a prior pointerdown, other keys and non-primary buttons change nothing", () => {
    // Arrange
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 700 })
    render(<Harness side="left" />)
    // Act
    fireEvent.pointerMove(handle(), { clientX: 900, pointerId: 1 })
    fireEvent.pointerDown(handle(), { button: 2, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 900, pointerId: 1 })
    fireEvent.keyDown(handle(), { key: "ArrowUp" })
    // Assert
    expect(width()).toBe(500)
  })
})

describe("ViewerResizeHandle — limits", () => {
  it("T-ui-vresize-03 [Boundary] right: the gallery keeps at least 288px (max = pane + main - 288); left: the right column keeps the collapsed TopBar minimum 288px (= gallery minimum)", () => {
    // Arrange: 왼쪽 — pane 500 + 오른쪽 열 700 = 1200 → 최대 912 (TopBar 접힌 최소 = 갤러리 최소 288)
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 700 })
    const { unmount } = render(<Harness side="left" />)
    // Act
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 5000, pointerId: 1 })
    // Assert
    expect(width()).toBe(912)
    unmount()
    // Act: 오른쪽 배치 — 같은 합계, 한참 왼쪽으로 끌기
    useViewStore.setState({ viewerWidth: 500 })
    stubGeometry({ paneLeft: 700, paneWidth: 500, galleryWidth: 700 })
    render(<Harness side="right" />)
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: -4000, pointerId: 1 })
    // Assert
    expect(width()).toBe(912)
  })

  it("T-ui-vresize-04 [Boundary] the viewer never shrinks below its minimum width", () => {
    // Arrange
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 700 })
    render(<Harness side="left" />)
    // Act
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 10, pointerId: 1 })
    // Assert
    expect(width()).toBe(VIEWER_MIN_WIDTH)
  })

  it("T-ui-vresize-06 [Boundary] without a main element or without layout (0 width) the max is unbounded, not NaN", () => {
    // Arrange: main 없음
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 700 })
    const { unmount } = render(<Harness side="left" withMain={false} />)
    // Act
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 5000, pointerId: 1 })
    // Assert
    expect(width()).toBe(5000)
    unmount()
    // Arrange: 레이아웃 없음(모두 0)
    stubGeometry({ paneLeft: 0, paneWidth: 0, galleryWidth: 0 })
    render(<Harness side="left" />)
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 700, pointerId: 1 })
    expect(width()).toBe(700)
  })

  it("T-ui-vresize-07 [Business rule] left: an open sidebar widens the right column's minimum (gallery 288 + sidebar) when that exceeds the TopBar minimum", () => {
    // Arrange: pane 500 + 열 900 = 1400, 사이드바 400 → 최소 max(288, 288+400=688) → 최대 712
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 900, navWidth: 400 })
    render(<Harness side="left" />)
    // Act
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 5000, pointerId: 1 })
    // Assert
    expect(width()).toBe(712)
  })

  it("T-ui-vresize-08 [Regression] left: the sibling column is found under the new structure, so the limit is finite for pointer and keyboard", () => {
    // Arrange: 구조 변경 전 방식(`:scope > main`)이면 Infinity 가 되어 제한이 사라졌다
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 900 })
    useViewStore.setState({ viewerWidth: 1112 })
    render(<Harness side="left" />)
    // Act
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 99999, pointerId: 1 })
    // Assert: 500 + 900 - 288
    expect(width()).toBe(1112)
    // Act: 키보드로 더 넓히기 시도
    fireEvent.keyDown(handle(), { key: "ArrowRight" })
    // Assert
    expect(width()).toBe(1112)
  })
})
