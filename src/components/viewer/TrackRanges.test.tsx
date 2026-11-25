// Categories: Happy (T-ui-ranges-10, 11, 12, 15, 16) / Validation (T-ui-ranges-05, 13, 18) / External (T-ui-ranges-06) / Business rule (T-ui-ranges-17, 19)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { RANGE_SAVE_DEBOUNCE_MS } from "@/hooks/usePlayRanges"
import { OPEN_LAYER } from "@/lib/openLayer"
import { advance, dragLane, fakeTimers, mountHtmlPlayer, selectRangeByContextMenu } from "@/test/ranges"

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

const RANGES = [
  { startMs: 10_000, endMs: 20_000 },
  { startMs: 40_000, endMs: 50_000 },
]

describe("integrated track ranges (HTML media)", () => {
  it("T-ui-ranges-10 [Happy] existing ranges render as buttons with times in their label; a left click on a box only seeks, right-click selects it (aria-pressed), a plain click on empty track deselects and seeks", async () => {
    // Arrange
    const { lane, el } = await mountHtmlPlayer({ ranges: RANGES })
    const first = await within(lane).findByRole("button", { name: "구간 1: 0:10.0 – 0:20.0" })
    const second = within(lane).getByRole("button", { name: "구간 2: 0:40.0 – 0:50.0" })
    expect(first).toHaveAttribute("aria-pressed", "false")
    // Act + Assert: 박스 위 왼쪽 클릭 = 탐색만 (선택 변화 없음)
    dragLane(second, 450, 450, lane)
    expect(el.currentTime).toBe(45)
    expect(second).toHaveAttribute("aria-pressed", "false")
    // Act + Assert: 우클릭 = 선택
    await selectRangeByContextMenu(second)
    expect(second).toHaveAttribute("aria-pressed", "true")
    expect(first).toHaveAttribute("aria-pressed", "false")
    // Act + Assert: 박스 위를 다시 눌러도 선택은 유지, 빈 곳 클릭 = 해제 + 탐색
    dragLane(second, 460, 460, lane)
    expect(second).toHaveAttribute("aria-pressed", "true")
    dragLane(lane, 700, 700)
    expect(second).toHaveAttribute("aria-pressed", "false")
    expect(el.currentTime).toBe(70)
    expect(screen.getByText("구간 2개")).toBeInTheDocument()
  })

  it("T-ui-ranges-11 [Happy] Shift+dragging on the track creates a range, saved once after the debounce; the server-normalized (merged) result replaces the local value", async () => {
    // Arrange: 서버는 맞닿은 구간을 병합해 돌려준다
    const { lane, set } = await mountHtmlPlayer({ ranges: RANGES })
    await within(lane).findByRole("button", { name: /구간 1/ })
    fakeTimers()
    // Act: 20.0s 에 맞닿게 (px 200 = 20s) 200→300 끌기 = 20.0~30.0s
    dragLane(lane, 200, 300, lane, true)
    // Assert: 낙관적 표시는 즉시, 저장은 디바운스 뒤
    expect(set).not.toHaveBeenCalled()
    await advance(RANGE_SAVE_DEBOUNCE_MS - 1)
    expect(set).not.toHaveBeenCalled()
    await advance(1)
    expect(set).toHaveBeenCalledTimes(1)
    expect(set).toHaveBeenCalledWith(1, [
      { startMs: 10_000, endMs: 30_000 },
      { startMs: 40_000, endMs: 50_000 },
    ])
    await advance(0)
    expect(within(lane).getByRole("button", { name: "구간 1: 0:10.0 – 0:30.0" })).toBeInTheDocument()
    expect(within(lane).getAllByRole("button")).toHaveLength(2)
  })

  it("T-ui-ranges-12 [Happy] selected range shows end handles; dragging the end handle adjusts the boundary and saves the integer ms", async () => {
    // Arrange
    const { lane, set } = await mountHtmlPlayer({ ranges: RANGES })
    const first = await within(lane).findByRole("button", { name: /구간 1/ })
    await selectRangeByContextMenu(first)
    fakeTimers()
    const handle = screen.getByTestId("range-handle-end")
    // Act: 끝 200px → 255px (= 25.5s)
    dragLane(handle, 200, 255, lane)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert
    expect(set).toHaveBeenCalledWith(1, [
      { startMs: 10_000, endMs: 25_500 },
      { startMs: 40_000, endMs: 50_000 },
    ])
    for (const call of set.mock.calls) for (const r of call[1]) expect(Number.isInteger(r.startMs) && Number.isInteger(r.endMs)).toBe(true)
  })

  it("T-ui-ranges-13 [Validation] dragging the start handle past the end is clamped to the minimum length (never inverted); a plain click on the track (seek) saves nothing", async () => {
    // Arrange: fps 25 → 최소 40ms
    const { lane, set } = await mountHtmlPlayer({ ranges: RANGES })
    await selectRangeByContextMenu(await within(lane).findByRole("button", { name: /구간 1/ }))
    fakeTimers()
    // Act 1: 시작 핸들을 끝(200px) 너머로
    dragLane(screen.getByTestId("range-handle-start"), 100, 400, lane)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 1
    const [, sent] = set.mock.calls[0]
    expect(sent[0].endMs).toBe(20_000)
    expect(sent[0].endMs - sent[0].startMs).toBe(40)
    // Act 2: 이동 없는 클릭은 탐색일 뿐 저장하지 않는다
    set.mockClear()
    dragLane(lane, 700, 700)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 2
    expect(set).not.toHaveBeenCalled()
  })

  it("T-ui-ranges-05 [Validation] a drag shorter than the minimum length is refused (toast, no save); a fractional drag position is rounded to integer ms before sending", async () => {
    // Arrange: 길이 1초 → 1px = 1ms, fps 25 → 최소 40ms
    const { lane, set } = await mountHtmlPlayer({ duration: 1, detail: { durationMs: 1000 } })
    fakeTimers()
    // Act 1: 20px = 20ms < 40ms
    dragLane(lane, 100, 120, lane, true)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 1
    expect(set).not.toHaveBeenCalled()
    expect(toast.warning).toHaveBeenCalledWith("구간이 너무 짧습니다 (최소 40ms).")
    // Act 2: 소수 좌표
    dragLane(lane, 100.4, 300.6, lane, true)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 2
    expect(set).toHaveBeenCalledTimes(1)
    const [, ranges] = set.mock.calls[0]
    expect(ranges).toEqual([{ startMs: 100, endMs: 301 }])
  })

  it("T-ui-ranges-06 [External] a failed save keeps the lane on the previous server ranges, shows a toast and an inline alert (aria role=alert)", async () => {
    // Arrange
    const { lane, set } = await mountHtmlPlayer({ ranges: RANGES })
    await within(lane).findByRole("button", { name: /구간 1/ })
    set.mockRejectedValueOnce({ code: "internal", message: "db" })
    fakeTimers()
    // Act
    dragLane(lane, 600, 700, lane, true)
    expect(within(lane).getAllByRole("button")).toHaveLength(3)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    await advance(0)
    // Assert
    expect(toast.error).toHaveBeenCalledWith("재생 구간을 저장하지 못했습니다. 이전 구간으로 되돌렸습니다.")
    expect(within(lane).getAllByRole("button")).toHaveLength(2)
    expect(screen.getByRole("alert")).toHaveTextContent("이전 구간으로 되돌렸습니다")
  })

  it("T-ui-ranges-17 [Business rule] edits made within the debounce window collapse into one save with the latest list; a pending edit is flushed when the viewer unmounts", async () => {
    // Arrange
    const { lane, set, view } = await mountHtmlPlayer({ ranges: RANGES })
    await within(lane).findByRole("button", { name: /구간 1/ })
    fakeTimers()
    // Act 1: 100ms 간격 두 번 편집 → 저장 1회
    dragLane(lane, 600, 700, lane, true)
    await advance(100)
    dragLane(lane, 800, 900, lane, true)
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 1
    expect(set).toHaveBeenCalledTimes(1)
    expect(set.mock.calls[0][1]).toHaveLength(4)
    // Act 2: 편집 직후 언마운트 → 즉시 전송
    set.mockClear()
    dragLane(lane, 920, 950, lane, true)
    expect(set).not.toHaveBeenCalled()
    view.unmount()
    await advance(0)
    // Assert 2
    expect(set).toHaveBeenCalledTimes(1)
  })
})

describe("integrated track context menu", () => {
  async function openMenu(lane: HTMLElement, name: RegExp) {
    const bar = await within(lane).findByRole("button", { name, hidden: true })
    fireEvent.contextMenu(bar)
    return screen.findByRole("menu")
  }

  it("T-ui-ranges-15 [Happy] right-click opens the menu (matches OPEN_LAYER so libVLC snapshots), selects the range, 'go to start' seeks, 'delete this range' saves the rest", async () => {
    // Arrange
    const user = userEvent.setup()
    const { lane, el, set } = await mountHtmlPlayer({ ranges: RANGES })
    // Act 1
    const menu = await openMenu(lane, /구간 2/)
    // Assert 1
    expect(document.querySelector(OPEN_LAYER)).not.toBeNull()
    // 메뉴가 열리면 나머지는 aria-hidden 이다
    expect(within(lane).getByRole("button", { name: /구간 2/, hidden: true })).toHaveAttribute("aria-pressed", "true")
    expect(within(menu).getByRole("menuitem", { name: /구간 시작으로 이동/ })).toBeInTheDocument()
    // Act 2: 시작으로 이동
    await user.click(within(menu).getByRole("menuitem", { name: /구간 시작으로 이동/ }))
    // Assert 2
    expect(el.currentTime).toBe(40)
    // Act 3: 삭제
    const menu2 = await openMenu(lane, /구간 2/)
    fakeTimers()
    fireEvent.click(within(menu2).getByRole("menuitem", { name: /이 구간 삭제/ }))
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert 3
    expect(set).toHaveBeenLastCalledWith(1, [{ startMs: 10_000, endMs: 20_000 }])
  })

  it("T-ui-ranges-16 [Happy] 'delete all…' asks in an AlertDialog; cancel keeps the ranges, confirm saves an empty list", async () => {
    // Arrange
    const user = userEvent.setup()
    const { lane, set } = await mountHtmlPlayer({ ranges: RANGES })
    const menu = await openMenu(lane, /구간 1/)
    // Act 1: 취소
    await user.click(within(menu).getByRole("menuitem", { name: /모든 구간 삭제/ }))
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("구간 2개가 삭제됩니다. 미디어 파일은 변경되지 않습니다.")
    await user.click(within(dialog).getByRole("button", { name: "취소" }))
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull())
    expect(set).not.toHaveBeenCalled()
    // Act 2: 확인
    const menu2 = await openMenu(lane, /구간 1/)
    fireEvent.click(within(menu2).getByRole("menuitem", { name: /모든 구간 삭제/ }))
    const confirm = await screen.findByRole("alertdialog")
    fakeTimers()
    fireEvent.click(within(confirm).getByRole("button", { name: "모두 삭제" }))
    await advance(RANGE_SAVE_DEBOUNCE_MS)
    // Assert
    expect(set).toHaveBeenLastCalledWith(1, [])
  })

  it("T-ui-ranges-18 [Validation] on empty space the per-range items are absent; \"delete all\" is always offered but disabled when there are no ranges", async () => {
    // Arrange
    const { lane } = await mountHtmlPlayer({ ranges: [] })
    // Act
    fireEvent.contextMenu(lane)
    const menu = await screen.findByRole("menu")
    // Assert
    expect(within(menu).queryByRole("menuitem", { name: /구간 시작으로 이동/ })).toBeNull()
    expect(within(menu).queryByRole("menuitem", { name: /이 구간 삭제/ })).toBeNull()
    expect(within(menu).getByRole("menuitem", { name: /모든 구간 삭제/ })).toHaveAttribute("aria-disabled", "true")
  })

  it("T-ui-ranges-19 [Business rule] menu key hints follow the selected preset (premiere)", async () => {
    // Arrange
    const { lane } = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangeKeys: "premiere" } })
    // Act
    const menu = await openMenu(lane, /구간 1/)
    // Assert
    expect(within(menu).getByRole("menuitem", { name: /구간 시작으로 이동/ })).toHaveTextContent("Shift+I")
    expect(within(menu).getByRole("menuitem", { name: /모든 구간 삭제/ })).toHaveTextContent("Ctrl+Shift+X")
    act(() => undefined)
  })
})
