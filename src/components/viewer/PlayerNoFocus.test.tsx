// Categories: Business rule (T-ui-nofocus-01, 02, 04, 05, 07: focus never moves on press / drag, Tab skips, keys keep working) / Happy (T-ui-nofocus-03 rate select by mouse) / Validation (T-ui-nofocus-06: viewer header is out of scope)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { mountMpv, mpvQueueItem } from "@/test/mpv"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.setPointerCapture ??= () => undefined
  Element.prototype.releasePointerCapture ??= () => undefined
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  document.body.querySelectorAll("[data-test-outside]").forEach((n) => n.remove())
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RANGES = [{ startMs: 10_000, endMs: 20_000 }]
const FOCUSABLES = 'button, [role="slider"], [role="combobox"]'

/** 플레이어 밖(갤러리 자리)에 포커스를 둔다 — "누르기 전 포커스" */
function focusOutside() {
  const outside = document.createElement("button")
  outside.setAttribute("data-test-outside", "")
  document.body.append(outside)
  outside.focus()
  expect(document.activeElement).toBe(outside)
  return outside
}

async function mount() {
  const t = await mountMpv({ ranges: RANGES })
  await t.ready()
  const lane = await screen.findByTestId("player-track")
  await within(lane).findByRole("button", { name: /구간 1/ })
  const controls = lane.closest<HTMLElement>("[data-player-controls]")
  if (!controls) throw new Error("controls root not found")
  // 슬라이더·트랙 값 계산이 0 폭으로 NaN 이 되지 않게 한다
  for (const el of [...controls.querySelectorAll("[data-slot=slider]"), lane.firstElementChild].filter((e): e is Element => e !== null)) {
    vi.spyOn(el, "getBoundingClientRect").mockReturnValue({ x: 0, y: 0, left: 0, top: 0, right: 100, bottom: 10, width: 100, height: 10, toJSON: () => ({}) })
  }
  return { t, lane, controls }
}

describe("player area controls never take focus", () => {
  it("T-ui-nofocus-01 [Business rule] pressing buttons in the control bar leaves document.activeElement unchanged", async () => {
    // Arrange
    const { controls } = await mount()
    const user = userEvent.setup()
    const outside = focusOutside()
    // Act + Assert
    for (const name of ["뒤로 (정지 시 1프레임)", "재생", "앞으로 (정지 시 1프레임)", "음소거"]) {
      await user.click(within(controls).getByRole("button", { name }))
      expect(document.activeElement, name).toBe(outside)
    }
    await user.click(within(controls).getByRole("button", { name: /구간만/ }))
    expect(document.activeElement).toBe(outside)
  })

  it("T-ui-nofocus-02 [Business rule] pressing / dragging the seek and volume sliders (Radix focuses the thumb itself) leaves focus where it was", async () => {
    // Arrange
    const { controls } = await mount()
    const outside = focusOutside()
    // Act + Assert: 누르기 → 끌기 → 놓기
    for (const name of ["재생 위치", "음량"]) {
      const thumb = within(controls).getByRole("slider", { name })
      // 재생 위치는 통합 트랙(커스텀), 음량은 Radix 슬라이더
      const root = name === "재생 위치" ? screen.getByTestId("player-track") : thumb.closest<HTMLElement>("[data-slot=slider]")
      if (!root) throw new Error("slider root not found")
      fireEvent.pointerDown(root, { clientX: 30, button: 0, pointerId: 1 })
      expect(document.activeElement, `${name} down`).toBe(outside)
      fireEvent.pointerMove(root, { clientX: 60, pointerId: 1 })
      expect(document.activeElement, `${name} move`).toBe(outside)
      fireEvent.pointerUp(root, { clientX: 60, pointerId: 1 })
      expect(document.activeElement, `${name} up`).toBe(outside)
    }
  })

  it("T-ui-nofocus-03 [Happy] the rate select still opens and selects by mouse, and focus returns to where it was after it closes", async () => {
    // Arrange
    const { t, controls } = await mount()
    const user = userEvent.setup()
    const outside = focusOutside()
    // Act
    await user.click(within(controls).getByRole("combobox", { name: "재생 속도" }))
    await user.click(await screen.findByRole("option", { name: "1.5×" }))
    // Assert
    await waitFor(() => expect(t.api.playerSetRate).toHaveBeenCalledWith(1.5))
    await waitFor(() => expect(screen.queryByRole("listbox")).not.toBeInTheDocument())
    await waitFor(() => expect(document.activeElement).toBe(outside))
  })

  it("T-ui-nofocus-04 [Business rule] range bars take clicks, right-click selection and track drags without taking focus", async () => {
    // Arrange
    const { lane } = await mount()
    const outside = focusOutside()
    const bar = within(lane).getByRole("button", { name: /구간 1/ })
    // Act: 클릭(탐색) → 우클릭으로 선택
    await userEvent.setup().click(bar)
    expect(document.activeElement).toBe(outside)
    fireEvent.contextMenu(bar)
    fireEvent.keyDown(await screen.findByRole("menu"), { key: "Escape" })
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    // Assert
    expect(bar).toHaveAttribute("aria-pressed", "true")
    expect(document.activeElement).toBe(outside)
    // Act: 빈 곳 끌기
    fireEvent.pointerDown(lane, { clientX: 300, button: 0, pointerId: 1 })
    fireEvent.pointerMove(lane, { clientX: 400, pointerId: 1 })
    fireEvent.pointerUp(lane, { clientX: 400, pointerId: 1 })
    expect(document.activeElement).toBe(outside)
  })

  it("T-ui-nofocus-05 [Business rule] Tab never lands on the control bar, lane, or queue; every control is tabindex -1", async () => {
    // Arrange: 연속 재생(바 + 대기열 포함)
    const t = await mountMpv({ ranges: RANGES, queue: [mpvQueueItem(1), mpvQueueItem(2)] })
    await t.ready()
    const user = userEvent.setup()
    const lane = await screen.findByTestId("player-track")
    await within(lane).findByRole("button", { name: /구간 1/ })
    const zones = [lane.closest("[data-player-controls]"), screen.getByRole("group", { name: "연속 재생" }), screen.getByRole("list", { name: "재생 대기열" })]
    // Assert 1: 영역 안 포커스 가능 요소는 전부 tabindex -1 (MutationObserver 반영 대기)
    await waitFor(() => {
      for (const zone of zones) {
        const items = zone?.querySelectorAll<HTMLElement>(FOCUSABLES) ?? []
        expect(items.length).toBeGreaterThan(0)
        for (const el of items) expect(el.tabIndex).toBe(-1)
      }
    })
    // Act + Assert 2: Tab 을 여러 번 눌러도 영역 안에 닿지 않는다
    focusOutside()
    for (let i = 0; i < 25; i++) {
      await user.tab()
      expect(zones.some((z) => z?.contains(document.activeElement))).toBe(false)
    }
  })

  it("T-ui-nofocus-07 [Business rule] playback bar and queue item presses keep focus; the player Space key still toggles play after pressing a control", async () => {
    // Arrange
    const t = await mountMpv({ ranges: RANGES, queue: [mpvQueueItem(1), mpvQueueItem(2)] })
    await t.ready()
    const user = userEvent.setup()
    const bar = screen.getByRole("group", { name: "연속 재생" })
    const outside = focusOutside()
    // Act + Assert: 바 버튼 · 대기열 항목
    await user.click(within(bar).getByRole("button", { name: "반복" }))
    expect(document.activeElement).toBe(outside)
    await user.click(within(screen.getByRole("list", { name: "재생 대기열" })).getAllByRole("button")[1])
    expect(document.activeElement).toBe(outside)
    // Act: 컨트롤을 누른 뒤 Space
    t.control.mockClear()
    await user.click(screen.getByRole("button", { name: "음소거" }))
    await user.keyboard(" ")
    // Assert: 플레이어 키 한 번만 (누른 버튼이 Space 를 먹지 않는다)
    expect(t.control).toHaveBeenCalledTimes(1)
    expect(t.control).toHaveBeenCalledWith("toggle")
  })

  it("T-ui-nofocus-06 [Validation] outside the player area (viewer header) focus behaves as before — the close button can take focus", async () => {
    // Arrange
    await mount()
    const header = screen.getByRole("button", { name: "보기 영역 닫기" })
    // Act
    act(() => header.focus())
    // Assert
    expect(document.activeElement).toBe(header)
  })
})
