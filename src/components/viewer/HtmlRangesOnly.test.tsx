// Categories: Business rule (T-ui-ranges-40..53) / Boundary (T-ui-ranges-45 D8, 46 D7) / External (T-ui-ranges-53)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { usePlaybackStore } from "@/stores"
import { mountHtmlPlayer, queueItem, restoreFrames, selectRangeByContextMenu } from "@/test/ranges"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  restoreFrames()
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RANGES = [
  { startMs: 10_000, endMs: 20_000 },
  { startMs: 40_000, endMs: 50_000 },
]
const queue = [queueItem(1), queueItem(2)]

/** 재생 위치를 정하고 컨트롤 바 상태에 반영한다 */
function at(el: HTMLMediaElement, seconds: number) {
  el.currentTime = seconds
  act(() => void el.dispatchEvent(new Event("timeupdate")))
}

describe("ranges-only on <video>/<audio>", () => {
  it("T-ui-ranges-40 [Business rule] the single-play toggle (and the count badge) is hidden with 0 ranges; with ranges it persists playRangesOnly and shows aria-pressed", async () => {
    // Arrange
    const empty = await mountHtmlPlayer()
    expect(screen.queryByRole("button", { name: "구간만" })).not.toBeInTheDocument()
    expect(screen.queryByText(/^구간 d+개$/)).not.toBeInTheDocument()
    empty.view.unmount()
    const t = await mountHtmlPlayer({ ranges: RANGES })
    const toggle = await screen.findByRole("button", { name: "구간만" })
    await waitFor(() => expect(toggle).toBeEnabled())
    expect(toggle).toHaveAttribute("aria-pressed", "false")
    // Act
    fireEvent.click(toggle)
    // Assert
    await waitFor(() => expect(screen.getByRole("button", { name: "구간만" })).toHaveAttribute("aria-pressed", "true"))
    expect((await t.api.getSettings()).playRangesOnly).toBe(true)
  })

  it("T-ui-ranges-41 [Business rule] with the toggle on, a fresh item parks at the first range start; playing inside a range does not seek", async () => {
    // Arrange + Act
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: true } })
    await waitFor(() => expect(t.el.currentTime).toBe(10))
    t.el.play()
    // Assert
    expect(t.el.currentTime).toBe(10)
    expect(t.frames.pendingVideo()).toBe(1)
  })

  it("T-ui-ranges-42 [Business rule] while playing, frames inside a range do nothing; reaching a range end (lookahead = half a frame) jumps to the next range start", async () => {
    // Arrange: fps 25 → lookahead 20ms
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: true } })
    await waitFor(() => expect(t.el.currentTime).toBe(10))
    t.el.play()
    // Act 1: 구간 안
    t.frames.frame(12)
    // Assert 1
    expect(t.el.currentTime).toBe(10)
    expect(t.frames.pendingVideo()).toBe(1)
    // Act 2: 19.98s + 20ms ≥ 20s → 다음 구간 (19.97s 는 아직 구간 안)
    t.frames.frame(19.97)
    expect(t.el.currentTime).toBe(10)
    t.frames.frame(19.98)
    // Assert 2
    expect(t.el.currentTime).toBe(40)
    expect(t.frames.pendingVideo()).toBe(1) // 계속 감시한다
    // Act 3: 다음 구간 안에서는 그대로
    t.frames.frame(40.5)
    expect(t.el.currentTime).toBe(40)
  })

  it("T-ui-ranges-43 [Business rule] the last range end pauses at the end position, stops watching and reports the end so a collection moves on", async () => {
    // Arrange: 연속 재생 + 컬렉션 구간만 켬
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { collectionPlayRangesOnly: true } })
    t.media.state.swallowFirstPlay = false // 연속 재생의 자동 재생 play() 를 센다
    act(() => usePlaybackStore.getState().start(1, queue))
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    await waitFor(() => expect(t.frames.pendingVideo()).toBe(1))
    // Act
    const video = document.querySelector("video") as HTMLVideoElement
    t.frames.frame(49.99)
    // Assert
    expect(t.media.pause).toHaveBeenCalled()
    expect(video.currentTime).toBe(50)
    expect(t.frames.pendingVideo()).toBe(0)
    await waitFor(() => expect(usePlaybackStore.getState().currentId).toBe(2))
  })

  it("T-ui-ranges-43b [Business rule] single play: the last range end pauses without any collection side effect", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: true } })
    await waitFor(() => expect(t.el.currentTime).toBe(10))
    t.el.play()
    // Act
    t.frames.frame(49.99)
    // Assert
    expect(t.el.currentTime).toBe(50)
    expect(t.el.paused).toBe(true)
    expect(usePlaybackStore.getState().status).toBe("idle")
    expect(t.frames.pendingVideo()).toBe(0)
  })

  it("T-ui-ranges-45 [Boundary] D8: playing from between ranges starts at the next range; from after the last range (e.g. replay after the end) restarts at the first range", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: true } })
    await waitFor(() => expect(t.el.currentTime).toBe(10))
    // Act 1: 구간 사이에서 재생
    at(t.el, 30)
    t.el.play()
    // Assert 1
    expect(t.el.currentTime).toBe(40)
    // Act 2: 마지막 구간 뒤에서 재생
    t.el.pause()
    at(t.el, 60)
    t.el.play()
    // Assert 2
    expect(t.el.currentTime).toBe(10)
    // Act 3: 끝에 멈춘 뒤 다시 재생
    t.frames.frame(49.99)
    expect(t.el.currentTime).toBe(50)
    t.el.play()
    expect(t.el.currentTime).toBe(10)
  })

  it("T-ui-ranges-46 [Boundary] D7: toggle on but no ranges → plays in full (no seek, no frame watching); ranges present but toggle off → also in full", async () => {
    // Arrange + Act 1: 구간 없음
    const none = await mountHtmlPlayer({ ranges: [], settings: { playRangesOnly: true } })
    none.el.play()
    // Assert 1
    expect(none.el.currentTime).toBe(0)
    expect(none.frames.pendingVideo()).toBe(0)
    none.view.unmount()
    // Arrange + Act 2: 구간은 있지만 토글 꺼짐
    const off = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: false } })
    await screen.findByRole("button", { name: /구간 1/ })
    off.el.play()
    // Assert 2
    expect(off.el.currentTime).toBe(0)
    expect(off.frames.pendingVideo()).toBe(0)
  })

  it("T-ui-ranges-47 [Business rule] pausing cancels the pending frame callback; turning the toggle on while already playing evaluates at once (jumps to the next range)", async () => {
    // Arrange: 토글 꺼짐, 구간 사이에서 재생 중
    const t = await mountHtmlPlayer({ ranges: RANGES })
    await screen.findByRole("button", { name: /구간 1/ })
    at(t.el, 30)
    t.el.play()
    expect(t.el.currentTime).toBe(30)
    // Act 1: 토글 켬 → 즉시 평가
    fireEvent.click(screen.getByRole("button", { name: "구간만" }))
    await waitFor(() => expect(t.el.currentTime).toBe(40))
    // Act 2: 일시정지 → 콜백 취소
    expect(t.frames.pendingVideo()).toBe(1)
    t.el.pause()
    // Assert
    expect(t.frames.pendingVideo()).toBe(0)
  })

  it("T-ui-ranges-48 [Business rule] <audio> uses requestAnimationFrame with a tight lookahead (8ms) and ends at the last range", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: true }, detail: { mediaType: "audio", fileName: "song.m4a", fps: null } })
    await waitFor(() => expect(t.el.currentTime).toBe(10))
    t.el.play()
    expect(t.frames.pendingVideo()).toBe(0)
    // Act 1: 구간 안
    t.el.currentTime = 12
    t.frames.tick()
    expect(t.el.currentTime).toBe(12)
    // Act 2: 19.995s + 8ms ≥ 20s → 다음 구간
    t.el.currentTime = 19.995
    t.frames.tick()
    expect(t.el.currentTime).toBe(40)
    // Act 3: 마지막 구간 끝
    t.el.currentTime = 49.995
    t.frames.tick()
    // Assert
    expect(t.el.currentTime).toBe(50)
    expect(t.el.paused).toBe(true)
  })

  it("T-ui-ranges-49 [Business rule] unmounting stops the frame watcher", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: true } })
    await waitFor(() => expect(t.el.currentTime).toBe(10))
    t.el.play()
    expect(t.frames.pendingVideo()).toBe(1)
    // Act
    t.view.unmount()
    // Assert
    expect(t.frames.pendingVideo()).toBe(0)
  })
})

describe("collection bar toggle and queue summary", () => {
  const summary = [{ assetId: 2, rangeCount: 2, totalMs: 20_000 }]

  it("T-ui-ranges-50 [Business rule] the playback bar toggle persists collectionPlayRangesOnly; the control-bar toggle is hidden during a collection; a single-play ON does not leak into the collection", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: RANGES, settings: { playRangesOnly: true, collectionPlayRangesOnly: false } })
    vi.spyOn(t.api, "collectionPlayRangeSummary").mockResolvedValue([])
    t.media.state.swallowFirstPlay = false // 연속 재생의 자동 재생 play() 를 센다
    act(() => usePlaybackStore.getState().start(1, queue))
    const bar = await screen.findByRole("group", { name: "연속 재생" })
    const toggle = within(bar).getByRole("button", { name: "구간만" })
    // Assert 1: 컬렉션 설정이 꺼져 있어 구간을 강제하지 않는다
    expect(toggle).toHaveAttribute("aria-pressed", "false")
    expect(screen.getAllByRole("button", { name: "구간만" })).toHaveLength(1)
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    expect(t.frames.pendingVideo()).toBe(0)
    // Act
    fireEvent.click(toggle)
    // Assert 2
    await waitFor(() => expect(within(screen.getByRole("group", { name: "연속 재생" })).getByRole("button", { name: "구간만" })).toHaveAttribute("aria-pressed", "true"))
    expect((await t.api.getSettings()).collectionPlayRangesOnly).toBe(true)
  })

  it("T-ui-ranges-51 [Business rule] D7 in a collection: an item without ranges plays in full even with the collection toggle on", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: [], settings: { collectionPlayRangesOnly: true } })
    vi.spyOn(t.api, "collectionPlayRangeSummary").mockResolvedValue([])
    // Act
    t.media.state.swallowFirstPlay = false // 연속 재생의 자동 재생 play() 를 센다
    act(() => usePlaybackStore.getState().start(1, queue))
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    // Assert
    expect(t.frames.pendingVideo()).toBe(0)
    expect(usePlaybackStore.getState().currentId).toBe(1)
  })

  it("T-ui-ranges-52 [Business rule] the queue lists 'n구간 · total' only for items that have ranges, fetched once per collection and again after a range edit is saved", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: RANGES })
    const spy = vi.spyOn(t.api, "collectionPlayRangeSummary").mockResolvedValue(summary)
    act(() => usePlaybackStore.getState().start(1, queue))
    const list = await screen.findByRole("list", { name: "재생 대기열" })
    // Assert 1
    expect(await within(list).findByText("2구간 · 0:20")).toBeInTheDocument()
    expect(within(list).getAllByText(/구간 ·/)).toHaveLength(1)
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith(1)
    // Act: 구간 삭제 저장 → 요약 다시 조회
    await selectRangeByContextMenu(await screen.findByRole("button", { name: /구간 1/ }))
    fireEvent.keyDown(document.body, { code: "Delete", key: "Delete" })
    // Assert 2
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(2), { timeout: 3000 })
  })

  it("T-ui-ranges-53 [External] a failing summary query leaves the queue intact without range labels", async () => {
    // Arrange
    const t = await mountHtmlPlayer({ ranges: [] })
    vi.spyOn(t.api, "collectionPlayRangeSummary").mockRejectedValue({ code: "internal", message: "x" })
    // Act
    act(() => usePlaybackStore.getState().start(1, queue))
    const list = await screen.findByRole("list", { name: "재생 대기열" })
    await act(async () => void (await Promise.resolve()))
    // Assert
    expect(within(list).getAllByRole("listitem")).toHaveLength(2)
    expect(within(list).queryByText(/구간 ·/)).toBeNull()
  })
})
