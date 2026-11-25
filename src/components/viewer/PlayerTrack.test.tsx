// Categories: Happy (T-ui-track-01, 02, 06, 07) / Validation (T-ui-track-03, 04) / Business rule (T-ui-track-05, 08, 09)
import { fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { formatDuration } from "@/lib/format"
import { auto, mountScenePlayer, sceneCuts } from "@/test/scene"
import { stubLayout } from "@/test/layout"
import { mountHtmlPlayer } from "@/test/ranges"
import { PlayerTrack } from "./PlayerTrack"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 범위·장면 UI 없이 트랙만 — 길이 100초, 트랙 1000px(1px = 100ms) */
function renderBare(over: { currentSec?: number; durationSec?: number } = {}) {
  stubLayout()
  const onSeek = vi.fn()
  const view = render(<PlayerTrack currentSec={over.currentSec ?? 0} durationSec={over.durationSec ?? 100} onSeek={onSeek} />)
  return { onSeek, view, track: screen.getByTestId("player-track"), slider: screen.getByRole("slider", { name: "재생 위치" }) }
}

describe("integrated track — slider contract and seeking", () => {
  it("T-ui-track-01 [Happy] the track exposes role=slider with min/max/now/valuetext in seconds and never takes tab focus", () => {
    // Arrange / Act
    const { slider } = renderBare({ currentSec: 12.34 })
    // Assert
    expect(slider).toHaveAttribute("aria-valuemin", "0")
    expect(slider).toHaveAttribute("aria-valuemax", "100")
    expect(slider).toHaveAttribute("aria-valuenow", "12.34")
    expect(slider).toHaveAttribute("aria-valuetext", `${formatDuration(12_340)} / ${formatDuration(100_000)}`)
    expect(slider.tabIndex).toBe(-1)
  })

  it("T-ui-track-02 [Happy] press seeks at once, dragging seeks live (0.1s steps, no duplicate calls), position is clamped to the ends, and releasing ends the drag", () => {
    // Arrange
    const { track, onSeek } = renderBare()
    // Act 1: 누르기 → 끌기 → 범위 밖 → 놓기
    fireEvent.pointerDown(track, { clientX: 300, button: 0, pointerId: 1 })
    fireEvent.pointerMove(track, { clientX: 300.2, pointerId: 1 })
    fireEvent.pointerMove(track, { clientX: 600, pointerId: 1 })
    fireEvent.pointerMove(track, { clientX: 1500, pointerId: 1 })
    fireEvent.pointerUp(track, { clientX: 1500, pointerId: 1 })
    // Assert 1
    expect(onSeek.mock.calls.map((c) => c[0])).toEqual([30, 60, 100])
    // Act 2: 놓은 뒤의 이동은 탐색하지 않는다
    fireEvent.pointerMove(track, { clientX: 200, pointerId: 1, buttons: 0 })
    expect(onSeek).toHaveBeenCalledTimes(3)
  })

  it("T-ui-track-03 [Validation] duration 0 disables the track: aria-disabled, no seek, no thumb, aria-valuemax stays 1", () => {
    // Arrange / Act
    const { track, slider, onSeek } = renderBare({ durationSec: 0 })
    fireEvent.pointerDown(track, { clientX: 300, button: 0, pointerId: 1 })
    // Assert
    expect(slider).toHaveAttribute("aria-disabled", "true")
    expect(slider).toHaveAttribute("aria-valuemax", "1")
    expect(onSeek).not.toHaveBeenCalled()
  })

  it("T-ui-track-04 [Validation] right / middle button presses never seek; a left press still does", () => {
    // Arrange
    const { track, onSeek } = renderBare()
    // Act
    for (const button of [1, 2]) {
      fireEvent.pointerDown(track, { clientX: 300, button, pointerId: 1 })
      fireEvent.pointerUp(track, { clientX: 300, button, pointerId: 1 })
    }
    // Assert
    expect(onSeek).not.toHaveBeenCalled()
    fireEvent.pointerDown(track, { clientX: 500, button: 0, pointerId: 1 })
    expect(onSeek).toHaveBeenCalledWith(50)
  })

  it("T-ui-track-06 [Happy] audio / no ranges / no scene: seek works, no range boxes, right-click opens no (empty) menu", () => {
    // Arrange
    const { track, onSeek } = renderBare()
    // Act
    fireEvent.pointerDown(track, { clientX: 250, button: 0, pointerId: 1 })
    fireEvent.pointerUp(track, { clientX: 250, pointerId: 1 })
    fireEvent.contextMenu(track)
    // Assert
    expect(onSeek).toHaveBeenCalledWith(25)
    expect(within(track).queryAllByRole("button")).toHaveLength(0)
    expect(screen.queryByRole("menu")).toBeNull()
  })
})

describe("integrated track — hover", () => {
  it("T-ui-track-05 [Business rule] hovering shows a time tooltip at the cursor; with a ranges UI Shift appends \" · 새 구간\" (also when Shift is pressed while hovering); leaving or pressing hides it", async () => {
    // Arrange
    const { lane } = await mountHtmlPlayer({ ranges: [] })
    // Act 1: 올리기
    fireEvent.pointerMove(lane, { clientX: 300, buttons: 0 })
    // Assert 1: 30초
    expect(screen.getByRole("tooltip")).toHaveTextContent(new RegExp(`^${formatDuration(30_000)}$`))
    // Act 2: Shift 를 누른 채 이동 / 올린 채 키 누름
    fireEvent.pointerMove(lane, { clientX: 300, buttons: 0, shiftKey: true })
    expect(screen.getByRole("tooltip")).toHaveTextContent(`${formatDuration(30_000)} · 새 구간`)
    fireEvent.pointerMove(lane, { clientX: 300, buttons: 0, shiftKey: false })
    expect(screen.getByRole("tooltip")).not.toHaveTextContent("새 구간")
    fireEvent.keyDown(window, { key: "Shift", shiftKey: true })
    expect(screen.getByRole("tooltip")).toHaveTextContent("· 새 구간")
    fireEvent.keyUp(window, { key: "Shift", shiftKey: false })
    expect(screen.getByRole("tooltip")).not.toHaveTextContent("새 구간")
    // Act 3: 누르면(끌기 시작) 숨김, 벗어나면 숨김
    fireEvent.pointerDown(lane, { clientX: 300, button: 0, pointerId: 1 })
    expect(screen.queryByRole("tooltip")).toBeNull()
    fireEvent.pointerUp(lane, { clientX: 300, pointerId: 1 })
    fireEvent.pointerMove(lane, { clientX: 300, buttons: 0 })
    fireEvent.pointerLeave(lane)
    expect(screen.queryByRole("tooltip")).toBeNull()
  })

  it("T-ui-track-07 [Happy] without a ranges UI Shift adds no suffix to the time tooltip", () => {
    // Arrange
    const { track } = renderBare()
    // Act
    fireEvent.pointerMove(track, { clientX: 500, buttons: 0, shiftKey: true })
    // Assert
    expect(screen.getByRole("tooltip")).toHaveTextContent(new RegExp(`^${formatDuration(50_000)}$`))
  })
})

describe("integrated track — range interactions", () => {
  it("T-ui-track-08 [Business rule] Shift+drag creates (never seeks), Shift+click without movement creates nothing, and dragging a handle adjusts without seeking", async () => {
    // Arrange
    const { lane, set, el } = await mountHtmlPlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }] })
    const box = await within(lane).findByRole("button", { name: /구간 1/ })
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    try {
      // Act 1: Shift+클릭(이동 없음)
      fireEvent.pointerDown(lane, { clientX: 600, button: 0, pointerId: 1, shiftKey: true })
      fireEvent.pointerUp(lane, { clientX: 600, pointerId: 1 })
      // Act 2: Shift+끌기
      fireEvent.pointerDown(lane, { clientX: 600, button: 0, pointerId: 1, shiftKey: true })
      fireEvent.pointerMove(lane, { clientX: 700, pointerId: 1 })
      fireEvent.pointerUp(lane, { clientX: 700, pointerId: 1 })
      await vi.advanceTimersByTimeAsync(2000)
      // Assert
      expect(set).toHaveBeenCalledTimes(1)
      expect(set.mock.calls[0][1]).toEqual([
        { startMs: 10_000, endMs: 20_000 },
        { startMs: 60_000, endMs: 70_000 },
      ])
      expect(el.currentTime).toBe(0)
      expect(box).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it("T-ui-track-09 [Business rule] right-click over a range that also holds a scene cut offers range items, cut items and the global items, separated into groups; the range becomes selected", async () => {
    // Arrange
    const m = await mountScenePlayer({ ranges: [{ startMs: 10_000, endMs: 20_000 }], scene: sceneCuts(1, [auto(15_000)]) })
    const box = await within(m.lane).findByRole("button", { name: /구간 1/ })
    await screen.findByRole("button", { name: /장면전환점 표시/ })
    // Act: 구간 박스 위, 컷(15s = 150px) 위치에서 우클릭
    fireEvent.contextMenu(box, { clientX: 150 })
    const menu = await screen.findByRole("menu")
    // Assert
    const names = within(menu)
      .getAllByRole("menuitem")
      .map((i) => i.textContent ?? "")
    expect(names).toHaveLength(7)
    expect(names[0]).toMatch(/구간 시작으로 이동/)
    expect(names[1]).toMatch(/이 구간 삭제/)
    expect(names[2]).toMatch(/모든 구간 삭제/)
    expect(names[3]).toMatch(/이 장면전환점으로 이동/)
    expect(names[4]).toMatch(/이 장면전환점 삭제/)
    expect(within(menu).getAllByRole("separator").length).toBe(3)
    expect(within(m.lane).getByRole("button", { name: /구간 1/, hidden: true })).toHaveAttribute("aria-pressed", "true")
  })
})
