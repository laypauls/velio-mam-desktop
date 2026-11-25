// Categories: Happy (T-ui-rangekeys-10, 11, 14) / Validation (T-ui-rangekeys-12, 15 ignore contexts, 16 Delete gating) / Business rule (T-ui-rangekeys-13, 17..20)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { RANGE_SAVE_DEBOUNCE_MS } from "@/hooks/usePlayRanges"
import { qk } from "@/hooks/keys"
import { advance, fakeTimers, mountHtmlPlayer } from "@/test/ranges"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type Mods = { ctrl?: boolean; alt?: boolean; shift?: boolean }
/** 물리 키 입력 — 한글 IME 상태는 key 만 다른 문자로 보낸다. 반환값 false = preventDefault 됨 */
function press(code: string, key: string, mods: Mods = {}, target: Element = document.body): boolean {
  return fireEvent.keyDown(target, { code, key, ctrlKey: !!mods.ctrl, altKey: !!mods.alt, shiftKey: !!mods.shift })
}
/** 재생 위치를 정하고 컨트롤 바 상태에 반영한다 */
function at(el: HTMLMediaElement, seconds: number) {
  el.currentTime = seconds
  act(() => void el.dispatchEvent(new Event("timeupdate")))
}

describe("play range keys on the player", () => {
  it("T-ui-rangekeys-10 [Happy] brackets: [ marks a pending In (dashed up to the playhead), ] at a later position confirms [In, now] and clears the pending mark; saved after the debounce", async () => {
    // Arrange
    const { el, lane, set } = await mountHtmlPlayer()
    at(el, 10)
    // Act 1
    press("BracketLeft", "[")
    at(el, 14)
    // Assert 1: 대기 In
    expect(within(lane).getByTestId("range-pending")).toBeInTheDocument()
    expect(screen.getByText("시작 0:10.0 표시됨 — 끝 키로 구간을 확정하세요.")).toBeInTheDocument()
    // Act 2
    fakeTimers()
    at(el, 20)
    press("BracketRight", "]")
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 2
    expect(set).toHaveBeenCalledWith(1, [{ startMs: 10_000, endMs: 20_000 }])
    expect(within(lane).queryByTestId("range-pending")).toBeNull()
    expect(within(lane).getByRole("button", { name: "구간 1: 0:10.0 – 0:20.0" })).toHaveAttribute("aria-pressed", "true")
  })

  it("T-ui-rangekeys-11 [Happy] Out first then In: ] marks a pending Out, [ at an earlier position confirms [now, Out]", async () => {
    // Arrange
    const { el, lane, set } = await mountHtmlPlayer()
    at(el, 30)
    press("BracketRight", "]")
    // Act
    fakeTimers()
    at(el, 25)
    press("BracketLeft", "[")
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert
    expect(set).toHaveBeenCalledWith(1, [{ startMs: 25_000, endMs: 30_000 }])
    expect(within(lane).queryByTestId("range-pending")).toBeNull()
  })

  it("T-ui-rangekeys-12 [Validation] Out at or before In is refused with a warning and keeps the pending In; a range shorter than one frame is refused too (nothing saved)", async () => {
    // Arrange: fps 25 → 최소 40ms
    const { el, set } = await mountHtmlPlayer()
    at(el, 20)
    press("BracketLeft", "[")
    // Act 1: 더 이른 위치에서 Out
    at(el, 10)
    press("BracketRight", "]")
    // Assert 1
    expect(toast.warning).toHaveBeenLastCalledWith("구간의 끝이 시작보다 뒤여야 합니다.")
    expect(screen.getByText(/시작 0:20.0 표시됨/)).toBeInTheDocument()
    // Act 2: 20ms 뒤
    fakeTimers()
    at(el, 20.02)
    press("BracketRight", "]")
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 2
    expect(toast.warning).toHaveBeenLastCalledWith("구간이 너무 짧습니다 (최소 40ms).")
    expect(set).not.toHaveBeenCalled()
  })

  it("T-ui-rangekeys-13 [Business rule] Alt+[ clears the pending In and Alt+] clears the pending Out, so the next ] / [ only marks again", async () => {
    // Arrange
    const { el, lane, set } = await mountHtmlPlayer()
    at(el, 10)
    press("BracketLeft", "[")
    expect(screen.getByText(/시작 0:10.0 표시됨/)).toBeInTheDocument()
    // Act 1
    press("BracketLeft", "[", { alt: true })
    // Assert 1
    expect(within(lane).queryByTestId("range-pending")).toBeNull()
    expect(screen.queryByText(/표시됨/)).toBeNull()
    // Act 2: 지운 뒤의 ] 는 확정이 아니라 대기 Out 표시
    fakeTimers()
    at(el, 20)
    press("BracketRight", "]")
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 2
    expect(set).not.toHaveBeenCalled()
    expect(screen.getByText(/끝 0:20.0 표시됨/)).toBeInTheDocument()
    press("BracketRight", "]", { alt: true })
    expect(screen.queryByText(/표시됨/)).toBeNull()
  })

  it("T-ui-rangekeys-14 [Happy] premiere preset with Korean IME: I / O (key 'ㅑ' / 'ㅐ', code KeyI / KeyO) create a range; the bracket keys do nothing", async () => {
    // Arrange
    const { el, set } = await mountHtmlPlayer({ settings: { playRangeKeys: "premiere" } })
    at(el, 5)
    press("BracketLeft", "[")
    expect(screen.queryByText(/표시됨/)).toBeNull()
    // Act
    press("KeyI", "ㅑ")
    fakeTimers()
    at(el, 8)
    press("KeyO", "ㅐ")
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert
    expect(set).toHaveBeenCalledWith(1, [{ startMs: 5000, endMs: 8000 }])
  })

  it("T-ui-rangekeys-15 [Validation] keys are ignored while typing in an input, while a dialog is open, and while the context menu is open", async () => {
    // Arrange
    const { el, lane } = await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }] })
    await within(lane).findByRole("button", { name: /구간 1/ })
    at(el, 30)
    // Act 1: 입력 칸
    const input = screen.getByLabelText("아무 입력")
    input.focus()
    const inInput = press("BracketLeft", "[", {}, input)
    // Assert 1
    expect(inInput).toBe(true)
    expect(screen.queryByText(/표시됨/)).toBeNull()
    // Act 2: 열린 메뉴
    fireEvent.contextMenu(within(lane).getByRole("button", { name: /구간 1/ }))
    await screen.findByRole("menu")
    const inMenu = press("BracketLeft", "[")
    // Assert 2
    expect(inMenu).toBe(true)
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape", code: "Escape" })
    // Act 3: 확인 대화상자
    press("KeyX", "X", { ctrl: true, shift: true })
    const dialog = await screen.findByRole("alertdialog")
    const inDialog = press("BracketLeft", "[")
    // Assert 3
    expect(dialog).toBeInTheDocument()
    expect(inDialog).toBe(true)
    expect(screen.queryByText(/표시됨/)).toBeNull()
  })

  it("T-ui-rangekeys-16 [Validation] Delete is taken only when a range is selected — otherwise it is not intercepted (other handlers keep it)", async () => {
    // Arrange
    const { lane, set } = await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }, { startMs: 40_000, endMs: 50_000 }] })
    await within(lane).findByRole("button", { name: /구간 1/ })
    // Act 1: 선택 없음
    const untouched = press("Delete", "Delete")
    // Assert 1: preventDefault 되지 않음(true), 저장 없음
    expect(untouched).toBe(true)
    // Act 2: 구간 1 선택(우클릭 → 메뉴 닫기) 후 Delete
    fireEvent.contextMenu(within(lane).getByRole("button", { name: /구간 1/ }))
    fireEvent.keyDown(await screen.findByRole("menu"), { key: "Escape" })
    await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
    fakeTimers()
    const taken = press("Delete", "Delete")
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 2
    expect(taken).toBe(false)
    expect(set).toHaveBeenCalledTimes(1)
    expect(set).toHaveBeenCalledWith(1, [{ startMs: 40_000, endMs: 50_000 }])
  })

  it("T-ui-rangekeys-17 [Business rule] Shift+[ / Shift+] jump to the previous range start / next range end", async () => {
    // Arrange
    const { el } = await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }, { startMs: 40_000, endMs: 50_000 }] })
    await screen.findByRole("button", { name: /구간 1/ })
    at(el, 45)
    // Act + Assert: 이전 구간 시작
    press("BracketLeft", "{", { shift: true })
    expect(el.currentTime).toBe(40)
    at(el, 45)
    // 다음 구간 끝
    press("BracketRight", "}", { shift: true })
    expect(el.currentTime).toBe(50)
    // 끝을 지나면 더 갈 곳이 없어 움직이지 않는다
    at(el, 55)
    press("BracketRight", "}", { shift: true })
    expect(el.currentTime).toBe(55)
  })

  it("T-ui-rangekeys-18 [Business rule] backslash selects the range under the playhead (no range there → no selection change)", async () => {
    // Arrange
    const { el, lane } = await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }, { startMs: 40_000, endMs: 50_000 }] })
    await within(lane).findByRole("button", { name: /구간 1/ })
    // Act 1
    at(el, 45)
    press("Backslash", "\\")
    // Assert 1
    expect(within(lane).getByRole("button", { name: /구간 2/ })).toHaveAttribute("aria-pressed", "true")
    // Act 2: 구간 밖
    at(el, 30)
    press("Backslash", "\\")
    // Assert 2
    expect(within(lane).getByRole("button", { name: /구간 2/ })).toHaveAttribute("aria-pressed", "true")
  })

  it("T-ui-rangekeys-19 [Business rule] Ctrl+Shift+X asks before deleting all (confirm saves []); with no ranges nothing opens", async () => {
    // Arrange
    const empty = await mountHtmlPlayer()
    press("KeyX", "X", { ctrl: true, shift: true })
    expect(screen.queryByRole("alertdialog")).toBeNull()
    empty.view.unmount()
    const { lane, set } = await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }] })
    await within(lane).findByRole("button", { name: /구간 1/ })
    // Act
    press("KeyX", "X", { ctrl: true, shift: true })
    const dialog = await screen.findByRole("alertdialog")
    fakeTimers()
    fireEvent.click(within(dialog).getByRole("button", { name: "모두 삭제" }))
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert
    expect(set).toHaveBeenCalledWith(1, [])
  })

  it("T-ui-rangekeys-20 [Business rule] switching the preset in settings applies at once: the new keys work and the old ones stop", async () => {
    // Arrange
    const { el, api, queryClient } = await mountHtmlPlayer()
    at(el, 10)
    press("KeyI", "i")
    expect(screen.queryByText(/표시됨/)).toBeNull() // 기본은 괄호 프리셋 — I 는 동작하지 않는다
    // Act
    await api.updateSettings({ playRangeKeys: "premiere" })
    await act(async () => {
      await queryClient.invalidateQueries({ queryKey: qk.settings })
      await new Promise((r) => setTimeout(r, 0)) // react-query 의 관찰자 알림(setTimeout 0)까지 반영
    })
    press("BracketLeft", "[")
    // Assert: 이전 키는 멈추고 새 키는 동작
    expect(screen.queryByText(/표시됨/)).toBeNull()
    press("KeyI", "i")
    expect(screen.getByText(/시작 0:10.0 표시됨/)).toBeInTheDocument()
  })
})
