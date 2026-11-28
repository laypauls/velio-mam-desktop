// Categories: Happy (T-ui-vresize-01, 02, 05) / Boundary (T-ui-vresize-03 gallery min clamp, 04 viewer min clamp, 06 no sibling) / Business rule (T-ui-vresize-07) / Regression (T-ui-vresize-08)
import { fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { resetAllStores, useViewStore } from "@/stores"
import { VIEWER_MIN_WIDTH } from "@/stores/view"
import { ViewerResizeHandle } from "./ViewerResizeHandle"
import { useRef } from "react"

/** 레이아웃 스텁 값 — 본문 행: [viewer 500] [handle] [main 600] (왼쪽 배치) / [main 600] [handle] [viewer 500] (오른쪽 배치). 보기 영역 폭 계산은 형제 main 만 본다 */
interface Geometry {
  paneLeft: number
  paneWidth: number
  /** 형제 `main`(갤러리) 폭 — 왼쪽·오른쪽 배치 공통 */
  galleryWidth: number
}

function rect(left: number, width: number): DOMRect {
  return { x: left, y: 0, top: 0, left, right: left + width, bottom: 100, width, height: 100, toJSON: () => ({}) } as DOMRect
}

function stubGeometry(g: Geometry) {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    if (this.tagName === "ASIDE") return rect(g.paneLeft, g.paneWidth)
    if (this.tagName === "MAIN") return rect(0, g.galleryWidth)
    return rect(0, 0)
  })
}

/** withMain=false 면 형제 `main` 을 만들지 않는다 */
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
      {withMain && side === "left" && <main />}
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
    // Arrange: 보기 영역 x 0..500, 갤러리 900 (최대 900+500-288 = 1112)
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
  it("T-ui-vresize-03 [Boundary] both sides: the gallery keeps at least 288px (max = pane + main - 288)", () => {
    // Arrange: 왼쪽 — pane 500 + main 700 = 1200 → 최대 912
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

  it("T-ui-vresize-06 [Boundary] without a sibling main (e.g. filmstrip) or without layout (0 width) the max is unbounded, not NaN", () => {
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

  it("T-ui-vresize-07 [Business rule] left uses the same limit as right — an open sidebar panel or TopBar does not change it (only pane + main - 288)", () => {
    // Arrange: pane 500 + main 900 = 1400 → 최대 1112. 패널·상세는 이 합계 밖이라 영향이 없다
    stubGeometry({ paneLeft: 0, paneWidth: 500, galleryWidth: 900 })
    render(<Harness side="left" />)
    // Act
    fireEvent.pointerDown(handle(), { button: 0, pointerId: 1 })
    fireEvent.pointerMove(handle(), { clientX: 5000, pointerId: 1 })
    // Assert
    expect(width()).toBe(1112)
  })

  it("T-ui-vresize-08 [Regression] left: the sibling main is found in the body row (no data-viewer-sibling column any more), so the limit is finite for pointer and keyboard", () => {
    // Arrange: 형제를 못 찾으면 Infinity 가 되어 제한이 사라진다
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
