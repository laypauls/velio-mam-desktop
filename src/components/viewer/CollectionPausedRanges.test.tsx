// Categories: Business rule (T-ui-ranges-54, 54b) / Boundary (paused vs stopped)
import { act, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { usePlaybackStore } from "@/stores"
import { mountHtmlPlayer, queueItem, restoreFrames } from "@/test/ranges"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  restoreFrames()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RANGES = [
  { startMs: 10_000, endMs: 20_000 },
  { startMs: 40_000, endMs: 50_000 },
]
const queue = [queueItem(1), queueItem(2)]
const SETTINGS = { playRangesOnly: false, collectionPlayRangesOnly: true }

describe("collection context while paused (regression: pause dropped the range enforcement)", () => {
  it("T-ui-ranges-54 [Business rule] pausing from the playback bar keeps the collection ranges-only setting: toggle stays pressed and hidden from the control bar, play still jumps into the first range", async () => {
    // Arrange: 단독 설정은 꺼짐, 컬렉션 설정만 켜짐
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: SETTINGS })
    vi.spyOn(t.api, "collectionPlayRangeSummary").mockResolvedValue([])
    act(() => usePlaybackStore.getState().start(1, queue))
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    const el = screen.getByLabelText<HTMLMediaElement>("clip.mp4", { selector: "video" })
    await waitFor(() => expect(el.currentTime).toBe(10))
    // Act
    act(() => usePlaybackStore.getState().togglePause())
    el.currentTime = 0
    act(() => void el.dispatchEvent(new Event("play")))
    // Assert
    expect(usePlaybackStore.getState().status).toBe("paused")
    const bar = await screen.findByRole("group", { name: "연속 재생" })
    expect(within(bar).getByRole("button", { name: "구간만" })).toHaveAttribute("aria-pressed", "true")
    expect(screen.getAllByRole("button", { name: "구간만" })).toHaveLength(1)
    expect(el.currentTime).toBe(10) // 구간 강제가 살아 있어 첫 구간 시작으로 이동
  })

  it("T-ui-ranges-54b [Boundary] once the queue stops (neither playing nor paused) the item falls back to the single-play setting", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: SETTINGS })
    vi.spyOn(t.api, "collectionPlayRangeSummary").mockResolvedValue([])
    act(() => usePlaybackStore.getState().start(1, queue))
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    // Act
    act(() => usePlaybackStore.setState({ status: "stopped" }))
    // Assert: 단독 재생 토글이 컨트롤 바에 나타나고 꺼짐으로 보인다
    const bar = screen.getByRole("group", { name: "연속 재생" })
    await waitFor(() => expect(screen.getAllByRole("button", { name: "구간만" })).toHaveLength(2)) // 재생 바 + 컨트롤 바
    const controlBarToggle = screen.getAllByRole("button", { name: "구간만" }).find((b) => !bar.contains(b))
    expect(controlBarToggle).toHaveAttribute("aria-pressed", "false")
  })
})
