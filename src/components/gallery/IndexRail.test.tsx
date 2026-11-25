// Categories: Happy (T-ui-rail-01..05, 10) / Boundary (T-ui-rail-06, 12, 13, 57) / Business rule (T-ui-rail-07, 08, 55, 56)
// 레이아웃: 트랙 높이 500, 위쪽 0 (stubLayout). 타이머는 fake — 라벨 페이드(1.5s)를 결정적으로 확인한다.
import { act, fireEvent, render, screen, within } from "@testing-library/react"
import{ afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { IndexRailMode, SearchIndex, SearchIndexKind } from "@/lib/ipc-contract"
import { stubLayout } from "@/test/layout"
import { IndexRail, RAIL_IDLE_MS } from "./IndexRail"
import type { GalleryScrollController } from "./useGalleryViewport"

const H = 500

beforeEach(() => {
  vi.useFakeTimers()
  stubLayout({ width: 56, height: H })
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

function months(startYear: number, count: number, withNone = false): SearchIndex["sections"] {
  const out = Array.from({ length: count }, (_, i) => ({
    key: `${startYear + Math.floor(i / 12)}-${String((i % 12) + 1).padStart(2, "0")}`,
    label: null,
    count: 100,
  }))
  if (withNone) out.push({ key: "none", label: null, count: 100 })
  return out
}
const sectionsOf = (keys: string[]): SearchIndex["sections"] => keys.map((key) => ({ key, label: null, count: 100 }))

interface Opts {
  kind?: SearchIndexKind
  sections?: SearchIndex["sections"]
  paging?: "infinite" | "paged"
  fraction?: number
  mode?: IndexRailMode
  total?: number
}

function setup(o: Opts = {}) {
  const kind = o.kind ?? "month"
  const sections = o.sections ?? (kind === "none" ? sectionsOf(["all"]) : months(2024, 27, true))
  const sum = sections.reduce((a, s) => a + s.count, 0)
  const total = o.total ?? sum
  const starts = sections.reduce<number[]>((acc, s, i) => [...acc, acc[i] + s.count], [0])
  const listeners = new Set<() => void>()
  const scrollToFraction = vi.fn()
  const scrollToSection = vi.fn()
  const wheel = vi.fn()
  const paging = o.paging ?? "infinite"
  const make = (fraction: number): GalleryScrollController => ({
    mode: paging,
    total,
    index: { total, generation: 1, kind, sections },
    fraction,
    page: 2,
    pageCount: Math.ceil(total / 100),
    range: [200, 300],
    scrollToFraction,
    scrollToSection,
    scrollToItem: vi.fn(),
    sectionFraction: (i) => starts[i] / sum,
    firstItem: Math.min(total - 1, Math.floor(fraction * total)),
    lastItem: Math.min(total - 1, Math.floor(fraction * total) + 10),
    wheel,
    onScrollActivity: (cb) => {
      listeners.add(cb)
      return () => void listeners.delete(cb)
    },
  })
  const ui = (fraction: number, mode: IndexRailMode = o.mode ?? "auto") => <IndexRail controller={make(fraction)} mode={mode} pageSize={100} />
  const view = render(ui(o.fraction ?? 0))
  return {
    ...view,
    rerenderAt: (fraction: number, mode?: IndexRailMode) => view.rerender(ui(fraction, mode)),
    rail: () => screen.getByRole("scrollbar"),
    ticks: () => screen.getByTestId("rail-ticks"),
    scroll: () => act(() => listeners.forEach((cb) => cb())),
    advance: (ms: number) => act(() => void vi.advanceTimersByTime(ms)),
    scrollToFraction,
    scrollToSection,
    wheel,
  }
}

describe("labels per index kind", () => {
  it("T-ui-rail-01 [Happy] month kind labels each year once and the undated section as 날짜 없음", () => {
    // Arrange + Act
    setup({ kind: "month", sections: months(2024, 27, true) })
    // Assert
    const ticks = within(screen.getByTestId("rail-ticks"))
    expect(ticks.getByText("2024")).toBeInTheDocument()
    expect(ticks.getByText("2025")).toBeInTheDocument()
    expect(ticks.getByText("2026")).toBeInTheDocument()
    expect(ticks.getByText("날짜 없음")).toBeInTheDocument()
    expect(ticks.queryByText("2027")).not.toBeInTheDocument()
  })

  it("T-ui-rail-02 [Happy] initial kind shows #, A-Z uppercase, 기타 for both etc and etc_end, and the choseong", () => {
    // Arrange + Act
    setup({ kind: "initial", sections: sectionsOf(["sym", "a", "b", "etc", "ㄱ", "ㄴ", "etc_end"]) })
    // Assert
    const ticks = within(screen.getByTestId("rail-ticks"))
    for (const text of ["#", "A", "B", "ㄱ", "ㄴ"]) expect(ticks.getByText(text)).toBeInTheDocument()
    expect(ticks.getAllByText("기타")).toHaveLength(2)
    expect(ticks.queryByText("a")).not.toBeInTheDocument()
  })

  it("T-ui-rail-03 [Boundary] none kind has no labels; the bubble shows a percentage (and the page number when paged)", () => {
    // Arrange
    const infinite = setup({ kind: "none", total: 1000 })
    // Act
    fireEvent.mouseMove(infinite.rail(), { clientY: H / 2 })
    // Assert
    expect(within(screen.getByTestId("rail-ticks")).queryAllByText(/\S/)).toHaveLength(0)
    expect(within(screen.getByTestId("rail-bubble")).getByText("50%")).toBeInTheDocument()
    infinite.unmount()
    // paged: 쪽 번호까지
    const paged = setup({ kind: "none", total: 1000, paging: "paged" })
    fireEvent.mouseMove(paged.rail(), { clientY: H / 2 })
    expect(screen.getByTestId("rail-bubble")).toHaveTextContent("50%")
    expect(screen.getByTestId("rail-bubble")).toHaveTextContent("6쪽")
  })

  it("T-ui-rail-13 [Business rule, gate ON] indexRail=off hides labels and dots but keeps the position handle and bubble", () => {
    // Arrange + Act
    const r = setup({ mode: "off" })
    // Assert
    expect(r.rail()).toBeInTheDocument()
    expect(within(r.ticks()).queryByText("2024")).not.toBeInTheDocument()
    fireEvent.mouseMove(r.rail(), { clientY: 10 })
    expect(screen.getByTestId("rail-bubble")).toHaveTextContent("2024년 1월")
  })

  it("T-ui-rail-12 [Boundary] renders nothing when there are no items", () => {
    // Arrange + Act
    setup({ sections: [], total: 0 })
    // Assert
    expect(screen.queryByRole("scrollbar")).not.toBeInTheDocument()
  })
})

describe("hover bubble and aria contract", () => {
  it("T-ui-rail-04 [Happy] hovering shows the section label with its count; paged adds the page number", () => {
    // Arrange: 28 구간 × 100 = 2,800건. 위치 0.5 = 14번째 구간(2025-03)
    const r = setup({ paging: "paged" })
    // Act
    fireEvent.mouseMove(r.rail(), { clientY: H / 2 })
    // Assert
    const bubble = screen.getByTestId("rail-bubble")
    expect(bubble).toHaveTextContent("2025년 3월")
    expect(bubble).toHaveTextContent("100개")
    expect(bubble).toHaveTextContent("15쪽")
    fireEvent.mouseLeave(r.rail())
    expect(screen.queryByTestId("rail-bubble")).not.toBeInTheDocument()
  })

  it("T-ui-rail-05 [Happy] scrollbar exposes label, 0-100 range, current value and the current section as aria-valuetext", () => {
    // Arrange + Act: 위치 0.5 → 2025-03
    const r = setup({ fraction: 0.5 })
    // Assert
    const rail = r.rail()
    expect(rail).toHaveAccessibleName("목차 · 스크롤 위치")
    expect(rail).toHaveAttribute("aria-valuemin", "0")
    expect(rail).toHaveAttribute("aria-valuemax", "100")
    expect(rail).toHaveAttribute("aria-valuenow", "50")
    expect(rail).toHaveAttribute("aria-valuetext", "2025년 3월")
    r.rerenderAt(0.999)
    expect(r.rail()).toHaveAttribute("aria-valuetext", "날짜 없음")
  })

  it("T-ui-rail-10 [Happy] paged mode marks the current page span on the track; infinite mode does not", () => {
    // Arrange + Act
    const paged = setup({ paging: "paged" })
    expect(screen.getByTestId("rail-page-span")).toBeInTheDocument()
    expect(paged.rail()).toHaveAttribute("aria-valuetext", expect.stringContaining("3쪽"))
    paged.unmount()
    setup({ paging: "infinite" })
    // Assert
    expect(screen.queryByTestId("rail-page-span")).not.toBeInTheDocument()
  })
})

describe("pointer interaction", () => {
  it("T-ui-rail-06 [Happy] a click (< 3px) goes to the section head under the pointer, once, without scrolling by fraction", () => {
    // Arrange: 위치 0.5 + 2px → 구간 index 14 (2025-03)
    const r = setup()
    // Act
    fireEvent.mouseDown(r.rail(), { clientY: H / 2, button: 0 })
    fireEvent.mouseMove(window, { clientY: H / 2 + 2 })
    fireEvent.mouseUp(window, { clientY: H / 2 + 2 })
    // Assert
    expect(r.scrollToSection).toHaveBeenCalledTimes(1)
    expect(r.scrollToSection).toHaveBeenCalledWith(14)
    expect(r.scrollToFraction).not.toHaveBeenCalled()
  })

  it("T-ui-rail-07 [Business rule] infinite drag scrolls continuously by fraction and applies the final spot on release", () => {
    // Arrange
    const r = setup({ paging: "infinite" })
    // Act
    fireEvent.mouseDown(r.rail(), { clientY: 100, button: 0 })
    fireEvent.mouseMove(window, { clientY: 200 })
    fireEvent.mouseMove(window, { clientY: 300 })
    // Assert: 끄는 동안 연속 호출 (빠른 스크롤 = jump 아님)
    expect(r.scrollToFraction).toHaveBeenNthCalledWith(1, 0.4, { jump: false })
    expect(r.scrollToFraction).toHaveBeenNthCalledWith(2, 0.6, { jump: false })
    // Act + Assert: 놓으면 확정
    fireEvent.mouseUp(window, { clientY: 300 })
    expect(r.scrollToFraction).toHaveBeenLastCalledWith(0.6, { jump: true })
    expect(r.scrollToFraction).toHaveBeenCalledTimes(3)
    expect(r.scrollToSection).not.toHaveBeenCalled()
  })

  it("T-ui-rail-08 [Business rule] paged drag only previews (bubble with page number) and moves once on release", () => {
    // Arrange
    const r = setup({ paging: "paged" })
    // Act
    fireEvent.mouseDown(r.rail(), { clientY: 100, button: 0 })
    fireEvent.mouseMove(window, { clientY: 250 })
    fireEvent.mouseMove(window, { clientY: 400 })
    // Assert: 끄는 중에는 이동 없음, 말풍선만 (0.8 → 2,800건 중 2,240번째 → 23쪽)
    expect(r.scrollToFraction).not.toHaveBeenCalled()
    expect(screen.getByTestId("rail-bubble")).toHaveTextContent("23쪽")
    // Act + Assert
    fireEvent.mouseUp(window, { clientY: 400 })
    expect(r.scrollToFraction).toHaveBeenCalledTimes(1)
    expect(r.scrollToFraction).toHaveBeenCalledWith(0.8, { jump: true })
    expect(r.scrollToSection).not.toHaveBeenCalled()
  })

  it("T-ui-rail-14 [Boundary] exactly 3px of movement already counts as a drag", () => {
    // Arrange
    const r = setup()
    // Act
    fireEvent.mouseDown(r.rail(), { clientY: 100, button: 0 })
    fireEvent.mouseMove(window, { clientY: 103 })
    fireEvent.mouseUp(window, { clientY: 103 })
    // Assert
    expect(r.scrollToSection).not.toHaveBeenCalled()
    expect(r.scrollToFraction).toHaveBeenCalled()
  })

  it("T-ui-rail-15 [Boundary] a click on a kind-none rail seeks by fraction (there are no sections to snap to)", () => {
    // Arrange
    const r = setup({ kind: "none", total: 1000 })
    // Act
    fireEvent.mouseDown(r.rail(), { clientY: 125, button: 0 })
    fireEvent.mouseUp(window, { clientY: 125 })
    // Assert
    expect(r.scrollToSection).not.toHaveBeenCalled()
    expect(r.scrollToFraction).toHaveBeenCalledWith(0.25, { jump: true })
  })
})

describe("visibility (scroll activity, hover, idle fade)", () => {
  it("T-ui-rail-18 [Time] labels are hidden at rest, appear on scroll activity and fade after the idle time", () => {
    // Arrange
    const r = setup()
    expect(r.ticks()).toHaveAttribute("data-state", "hidden")
    // Act + Assert: 스크롤 → 표시
    r.scroll()
    expect(r.ticks()).toHaveAttribute("data-state", "visible")
    r.advance(RAIL_IDLE_MS - 1)
    expect(r.ticks()).toHaveAttribute("data-state", "visible")
    // 계속 스크롤하면 타이머가 다시 시작된다
    r.scroll()
    r.advance(RAIL_IDLE_MS - 1)
    expect(r.ticks()).toHaveAttribute("data-state", "visible")
    r.advance(1)
    expect(r.ticks()).toHaveAttribute("data-state", "hidden")
  })

  it("T-ui-rail-19 [Time] hover keeps labels visible and starts the fade only when it ends", () => {
    // Arrange
    const r = setup()
    // Act + Assert: 호버
    fireEvent.mouseMove(r.rail(), { clientY: 50 })
    r.advance(RAIL_IDLE_MS * 2)
    expect(r.ticks()).toHaveAttribute("data-state", "visible")
    fireEvent.mouseLeave(r.rail())
    expect(r.ticks()).toHaveAttribute("data-state", "visible")
    r.advance(RAIL_IDLE_MS)
    expect(r.ticks()).toHaveAttribute("data-state", "hidden")
  })

  it("T-ui-rail-20 [Business rule, gate ON] indexRail=always keeps labels visible at rest", () => {
    // Arrange + Act
    const r = setup({ mode: "always" })
    r.advance(RAIL_IDLE_MS * 3)
    // Assert
    expect(r.ticks()).toHaveAttribute("data-state", "visible")
    expect(within(r.ticks()).getByText("2025")).toBeInTheDocument()
  })

  it("T-ui-rail-21 [Concurrency] unmounting while the fade timer is pending raises no state update", () => {
    // Arrange
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined)
    const r = setup()
    r.scroll()
    // Act
    r.unmount()
    r.advance(RAIL_IDLE_MS)
    // Assert
    expect(spy).not.toHaveBeenCalled()
  })
})

describe("never takes focus", () => {
  /** 갤러리 스크롤 영역 대역 — 레일 앞뒤에 포커스 가능한 요소를 둔다 */
  function withGallery() {
    const r = setup()
    const gallery = document.createElement("div")
    gallery.tabIndex = 0
    gallery.id = "gallery-stub"
    document.body.append(gallery)
    return { r, gallery, cleanup: () => gallery.remove() }
  }

  it("T-ui-rail-55 [Business rule] the rail is not focusable: no tabindex (outside the Tab order) and focus() on it does nothing", () => {
    // Arrange: 레일이 DOM 에서 갤러리 대역보다 앞이라, 포커스 가능했다면 Tab 순서에 들어간다
    const { r, gallery, cleanup } = withGallery()
    gallery.focus()
    // Act
    act(() => r.rail().focus())
    // Assert
    expect(r.rail()).not.toHaveAttribute("tabindex")
    expect(r.rail().tabIndex).toBe(-1)
    expect(gallery).toHaveFocus()
    cleanup()
  })

  it("T-ui-rail-56 [Business rule] a click and a drag on the rail leave the existing focus where it was", () => {
    // Arrange
    const { r, gallery, cleanup } = withGallery()
    gallery.focus()
    // Act + Assert: 클릭 — mousedown 의 기본 동작(포커스 이동)을 막는다
    const downPrevented = !fireEvent.mouseDown(r.rail(), { clientY: 100, button: 0 })
    fireEvent.mouseUp(window, { clientY: 100 })
    expect(downPrevented).toBe(true)
    expect(gallery).toHaveFocus()
    // Act + Assert: 드래그
    fireEvent.mouseDown(r.rail(), { clientY: 100, button: 0 })
    fireEvent.mouseMove(window, { clientY: 250 })
    fireEvent.mouseUp(window, { clientY: 250 })
    expect(gallery).toHaveFocus()
    cleanup()
  })

  it("T-ui-rail-57 [Boundary] keys dispatched on the rail do nothing (no rail keyboard navigation)", () => {
    // Arrange
    const r = setup()
    // Act
    const notPrevented = fireEvent.keyDown(r.rail(), { key: "ArrowDown" })
    // Assert: 키보드 조작은 갤러리 몫 — 레일은 키를 가로채지 않는다
    expect(notPrevented).toBe(true)
    expect(r.scrollToSection).not.toHaveBeenCalled()
    expect(r.scrollToFraction).not.toHaveBeenCalled()
  })
})

describe("name index (equal-spacing slots) interaction", () => {
  const KEYS = ["sym", "a", "b", "ㄱ", "etc_end"] // 구간 5개 × 100건, 트랙 500px: 칸 y = 6, 128, 250, 372, 494

  it("T-ui-rail-23 [Happy] infinite drag jumps section by section only when the section under the pointer changes; release in the same slot sends nothing more", () => {
    // Arrange
    const r = setup({ kind: "initial", sections: sectionsOf(KEYS) })
    // Act
    fireEvent.mouseDown(r.rail(), { clientY: 100, button: 0 })
    fireEvent.mouseMove(window, { clientY: 250 })
    fireEvent.mouseMove(window, { clientY: 300 }) // 같은 칸
    fireEvent.mouseMove(window, { clientY: 400 })
    fireEvent.mouseUp(window, { clientY: 400 })
    // Assert
    expect(r.scrollToSection.mock.calls.map((c) => c[0])).toEqual([2, 3])
    expect(r.scrollToFraction).not.toHaveBeenCalled()
  })

  it("T-ui-rail-24 [Business rule] paged drag only previews (section + page of its first item) and applies once on release", () => {
    // Arrange
    const r = setup({ kind: "initial", sections: sectionsOf(KEYS), paging: "paged" })
    // Act
    fireEvent.mouseDown(r.rail(), { clientY: 100, button: 0 })
    fireEvent.mouseMove(window, { clientY: 250 })
    fireEvent.mouseMove(window, { clientY: 400 })
    // Assert: 끄는 중 이동 없음, 포인터 아래 구간(ㄱ = 301번째 항목 → 4쪽)을 말풍선이 알려 준다
    expect(r.scrollToSection).not.toHaveBeenCalled()
    expect(screen.getByTestId("rail-bubble")).toHaveTextContent("ㄱ")
    expect(screen.getByTestId("rail-bubble")).toHaveTextContent("4쪽")
    // Act + Assert
    fireEvent.mouseUp(window, { clientY: 400 })
    expect(r.scrollToSection).toHaveBeenCalledTimes(1)
    expect(r.scrollToSection).toHaveBeenCalledWith(3)
  })

  it("T-ui-rail-25 [Happy] a click picks the nearest slot regardless of item counts", () => {
    // Arrange: 첫 구간이 전체의 96% 여도 칸은 같은 크기
    const sections = sectionsOf(KEYS).map((s, i) => ({ ...s, count: i === 0 ? 9600 : 100 }))
    const r = setup({ kind: "initial", sections })
    // Act: 칸 2(y=250) 근처
    fireEvent.mouseDown(r.rail(), { clientY: 262, button: 0 })
    fireEvent.mouseUp(window, { clientY: 262 })
    // Assert
    expect(r.scrollToSection).toHaveBeenCalledWith(2)
  })

  it("T-ui-rail-26 [Happy] the wheel over the rail goes to the controller, is prevented from scrolling the page, and shows the labels", () => {
    // Arrange
    const r = setup()
    expect(r.ticks()).toHaveAttribute("data-state", "hidden")
    // Act
    const notPrevented = fireEvent.wheel(r.rail(), { deltaY: 120 })
    // Assert: 컨트롤러의 휠 처리기가 받는다(여기서는 스텁이라 preventDefault 는 그쪽 몫) — 레일은 비수동으로 연결하고 라벨을 띄운다
    expect(r.wheel).toHaveBeenCalledTimes(1)
    expect((r.wheel.mock.calls[0][0] as WheelEvent).deltaY).toBe(120)
    expect(notPrevented).toBe(true)
    expect(r.ticks()).toHaveAttribute("data-state", "visible")
  })
})
