// Categories: Happy (T-ui-ctrl-01 range buttons, 05 scene nav click, 08 nav click) / Boundary (T-ui-ctrl-02 0 ranges, 04 no cuts, 06 disabled ends, 07 no nav)
// / Business rule (T-ui-ctrl-03 parity with keys, 05 toggle off, 09 Shift+N/P, 10 playback queue, 11 rate labels, 12 no focus)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { RANGE_SAVE_DEBOUNCE_MS } from "@/hooks/usePlayRanges"
import { usePlaybackStore, useViewerNavStore } from "@/stores"
import { advance, fakeTimers, mountHtmlPlayer, queueItem } from "@/test/ranges"
import { auto, mountScenePlayer, sceneCuts, user as userCut } from "@/test/scene"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.setPointerCapture ??= () => undefined
  Element.prototype.releasePointerCapture ??= () => undefined
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  document.body.querySelectorAll("[data-test-outside]").forEach((n) => n.remove())
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 재생 위치를 정하고 컨트롤 바 상태에 반영한다 */
function at(el: HTMLMediaElement, seconds: number) {
  el.currentTime = seconds
  act(() => void el.dispatchEvent(new Event("timeupdate")))
}
const key = (code: string, k: string, mods: { ctrl?: boolean; shift?: boolean } = {}) =>
  fireEvent.keyDown(document.body, { code, key: k, ctrlKey: !!mods.ctrl, shiftKey: !!mods.shift })
const btn = (name: string | RegExp) => screen.queryByRole("button", { name })

describe("range group", () => {
  it("T-ui-ctrl-01 [Happy] [시작][끝] buttons run the same markIn/markOut as the keys (pending → confirmed, saved) and the tooltip shows the preset key", async () => {
    // Arrange
    const { el, set } = await mountHtmlPlayer()
    const markIn = screen.getByRole("button", { name: "구간 시작 표시" })
    const markOut = screen.getByRole("button", { name: "구간 끝 표시" })
    expect(markIn).toHaveAttribute("title", "구간 시작 표시 [[]")
    expect(markOut).toHaveAttribute("title", "구간 끝 표시 []]")
    // Act
    at(el, 10)
    fireEvent.click(markIn)
    expect(markIn).toHaveAttribute("aria-pressed", "true")
    fakeTimers()
    at(el, 20)
    fireEvent.click(markOut)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert
    expect(set).toHaveBeenCalledWith(1, [{ startMs: 10_000, endMs: 20_000 }])
    expect(markIn).toHaveAttribute("aria-pressed", "false")
  })

  it("T-ui-ctrl-02 [Boundary] 0 ranges: [시작][끝] stay but the count badge and 구간만 are hidden; with ranges both appear", async () => {
    // Arrange / Act
    const empty = await mountHtmlPlayer()
    // Assert
    expect(btn("구간 시작 표시")).toBeInTheDocument()
    expect(btn("구간 끝 표시")).toBeInTheDocument()
    expect(btn("구간만")).toBeNull()
    expect(screen.queryByText(/^구간 \d+개$/)).toBeNull()
    empty.view.unmount()
    await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }] })
    expect(await screen.findByText("구간 1개")).toBeInTheDocument()
    expect(btn("구간만")).toBeInTheDocument()
  })
})

describe("scene group prev/next", () => {
  const CUTS = [auto(10_000), userCut(20_000), auto(30_000)]
  const mount = async (settings = {}) => {
    const m = await mountScenePlayer({ scene: sceneCuts(1, CUTS), settings })
    await screen.findByRole("button", { name: /장면전환점 표시/ })
    return m
  }

  it("T-ui-ctrl-03 [Business rule] 이전/다음 장면 buttons land exactly where Ctrl+←/→ do (cut + half frame), and are disabled when there is nothing ahead/behind", async () => {
    // Arrange
    const { el } = await mount()
    const prev = screen.getByRole("button", { name: "이전 장면" })
    const next = screen.getByRole("button", { name: "다음 장면" })
    expect(prev).toHaveAttribute("title", "이전 장면 [Ctrl+←]")
    // 첫 컷 앞: 이전 비활성
    at(el, 5)
    expect(prev).toBeDisabled()
    expect(next).toBeEnabled()
    // Act 1: 버튼
    fireEvent.click(next)
    const byButton = el.currentTime
    // Act 2: 같은 상태에서 키 (직전 이동 목표 1초 창이 지난 뒤)
    const now = Date.now()
    vi.spyOn(Date, "now").mockReturnValue(now + 5000)
    at(el, 5)
    key("ArrowRight", "ArrowRight", { ctrl: true })
    // Assert
    expect(byButton).toBeCloseTo(10.02, 3)
    expect(el.currentTime).toBe(byButton)
    // 마지막 컷 뒤: 다음 비활성
    at(el, 40)
    vi.spyOn(Date, "now").mockReturnValue(now + 10_000)
    expect(next).toBeDisabled()
    expect(prev).toBeEnabled()
    fireEvent.click(prev)
    expect(el.currentTime).toBeCloseTo(30.02, 3)
  })

  it("T-ui-ctrl-04 [Boundary] no cuts (and no extract state with cuts) → no prev/next buttons", async () => {
    // Arrange / Act
    await mountScenePlayer({ scene: sceneCuts(1, []) })
    await screen.findByRole("button", { name: "장면 추출" })
    // Assert
    expect(btn("이전 장면")).toBeNull()
    expect(btn("다음 장면")).toBeNull()
  })

  it("T-ui-ctrl-05 [Business rule, gate ON] with the visibility toggle off the buttons still move (like Ctrl+←/→)", async () => {
    // Arrange: sceneCutsVisible=false
    const { el } = await mount({ sceneCutsVisible: false })
    expect(screen.getByRole("button", { name: /장면전환점 표시/ })).toHaveAttribute("aria-pressed", "false")
    at(el, 15)
    // Act
    fireEvent.click(screen.getByRole("button", { name: "다음 장면" }))
    // Assert
    expect(el.currentTime).toBeCloseTo(20.02, 3)
  })
})

describe("previous / next content", () => {
  it("T-ui-ctrl-07 [Boundary] no source (not opened from a gallery, no playback) → no content buttons at all", async () => {
    await mountHtmlPlayer()
    expect(btn("이전 콘텐츠")).toBeNull()
    expect(btn("다음 콘텐츠")).toBeNull()
  })

  it("T-ui-ctrl-06 [Boundary] first item: 이전 is disabled (not hidden), 다음 is enabled; the tooltips carry Shift+P / Shift+N", async () => {
    // Arrange
    await mountHtmlPlayer()
    // Act
    act(() => useViewerNavStore.getState().setStepper({ canPrev: false, canNext: true, step: vi.fn() }))
    // Assert
    const prev = await screen.findByRole("button", { name: "이전 콘텐츠" })
    expect(prev).toBeDisabled()
    expect(prev).toHaveAttribute("title", "이전 콘텐츠 [Shift+P]")
    expect(screen.getByRole("button", { name: "다음 콘텐츠" })).toBeEnabled()
    expect(screen.getByRole("button", { name: "다음 콘텐츠" })).toHaveAttribute("title", "다음 콘텐츠 [Shift+N]")
  })

  it("T-ui-ctrl-08 [Happy] single view: the buttons call the gallery stepper with the direction", async () => {
    // Arrange
    await mountHtmlPlayer()
    const step = vi.fn()
    act(() => useViewerNavStore.getState().setStepper({ canPrev: true, canNext: true, step }))
    // Act
    fireEvent.click(await screen.findByRole("button", { name: "다음 콘텐츠" }))
    fireEvent.click(screen.getByRole("button", { name: "이전 콘텐츠" }))
    // Assert
    expect(step.mock.calls).toEqual([[1], [-1]])
  })

  it("T-ui-ctrl-09 [Business rule] single view: Shift+N / Shift+P step too, once per press, and a disabled direction does nothing", async () => {
    // Arrange: 첫 항목 — 이전 불가
    await mountHtmlPlayer()
    const step = vi.fn()
    act(() => useViewerNavStore.getState().setStepper({ canPrev: false, canNext: true, step }))
    await screen.findByRole("button", { name: "다음 콘텐츠" })
    // Act
    expect(key("KeyN", "N", { shift: true })).toBe(false) // consumed
    fireEvent.keyDown(document.body, { code: "KeyN", key: "N", shiftKey: true, repeat: true })
    key("KeyP", "P", { shift: true })
    // Assert
    expect(step.mock.calls).toEqual([[1]])
  })

  it("T-ui-ctrl-10 [Business rule, gate ON] continuous playback: the queue decides — prev disabled at the first item with repeat off, next calls the store's next (Shift+N is not doubled)", async () => {
    // Arrange
    await mountHtmlPlayer()
    const next = vi.fn()
    act(() => usePlaybackStore.setState({ status: "playing", queue: [queueItem(1), queueItem(2)], index: 0, currentId: 1, repeat: "off", next }))
    // Act
    const nextBtn = await screen.findByRole("button", { name: "다음 콘텐츠" })
    fireEvent.click(nextBtn)
    // Assert
    expect(screen.getByRole("button", { name: "이전 콘텐츠" })).toBeDisabled()
    expect(next).toHaveBeenCalledTimes(1)
    key("KeyN", "N", { shift: true })
    expect(next).toHaveBeenCalledTimes(2) // 키는 usePlaybackKeys 한 곳만 처리
    // 마지막 항목 + 반복 끔: 다음 비활성, 이전 활성
    act(() => usePlaybackStore.setState({ index: 1, currentId: 2 }))
    await waitFor(() => expect(screen.getByRole("button", { name: "다음 콘텐츠" })).toBeDisabled())
    expect(screen.getByRole("button", { name: "이전 콘텐츠" })).toBeEnabled()
    // 반복 켬: 양쪽 활성
    act(() => usePlaybackStore.setState({ repeat: "all", index: 0 }))
    await waitFor(() => expect(screen.getByRole("button", { name: "이전 콘텐츠" })).toBeEnabled())
  })
})

describe("rate and focus", () => {
  it("T-ui-ctrl-11 [Happy] the compact rate trigger shows 1× and the options read 0.25× … 2×", async () => {
    // Arrange
    await mountHtmlPlayer()
    const user = userEvent.setup()
    const trigger = screen.getByRole("combobox", { name: "재생 속도" })
    expect(trigger).toHaveTextContent("1×")
    // Act
    await user.click(trigger)
    // Assert
    const options = await screen.findAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual(["0.25×", "0.5×", "1×", "1.5×", "2×"])
  })

  it("T-ui-ctrl-12 [Business rule] pressing the new buttons (content nav, range in/out, scene prev/next) never moves focus", async () => {
    // Arrange
    const { el } = await mountScenePlayer({ scene: sceneCuts(1, [auto(10_000)]) })
    await screen.findByRole("button", { name: /장면전환점 표시/ })
    act(() => useViewerNavStore.getState().setStepper({ canPrev: true, canNext: true, step: vi.fn() }))
    at(el, 5)
    const outside = document.createElement("button")
    outside.setAttribute("data-test-outside", "")
    document.body.append(outside)
    outside.focus()
    const user = userEvent.setup()
    // Act + Assert
    for (const name of ["이전 콘텐츠", "다음 콘텐츠", "구간 시작 표시", "구간 끝 표시", "다음 장면"]) {
      await user.click(await screen.findByRole("button", { name }))
      expect(document.activeElement, name).toBe(outside)
    }
    expect(within(document.body).getByRole("button", { name: "다음 장면" })).toBeInTheDocument()
  })
})
