// Categories: Business rule (T-ui-autoplay-01..04 built-in player, T-ui-ranges-30..34 ranges-only) / Happy (T-ui-input-01, 02, 05) / Boundary (T-ui-input-03 fractional wheel) / Validation (T-ui-input-04 gating)
import { act, fireEvent, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { WHEEL_IDLE_MS } from "./wheelNotches"
import { advance, fakeTimers, selectRangeByContextMenu } from "@/test/ranges"
import { CONTAIN_1080P, mountMpv, mpvQueueItem } from "@/test/mpv"

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

describe("built-in player open follows the autoplay setting (D11)", () => {
  it("T-ui-autoplay-01 [Business rule] single open: setting off → no poster, the player opens paused (autoplay=false) showing the first frame; on → autoplay=true", async () => {
    // Arrange + Act: 꺼짐 — 포스터 없이 정지로 열린다
    const off = await mountMpv({ settings: { mediaOpenAutoplay: false } })
    await off.ready()
    // Assert
    expect(screen.queryByTestId("video-poster")).not.toBeInTheDocument()
    expect(off.open).toHaveBeenCalledWith(1, CONTAIN_1080P, false)
    expect(off.canvas()).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
    off.view.unmount()
    // Arrange + Act: 켬
    const on = await mountMpv({ settings: { mediaOpenAutoplay: true } })
    await on.ready()
    // Assert
    expect(on.open).toHaveBeenCalledWith(1, CONTAIN_1080P, true)
  })

  it("T-ui-autoplay-02 [Business rule] collection playback always opens with autoplay=true, even when the setting is off", async () => {
    // Arrange + Act
    const t = await mountMpv({ queue: [mpvQueueItem(1), mpvQueueItem(2)], settings: { mediaOpenAutoplay: false } })
    await t.ready()
    // Assert
    expect(t.open).toHaveBeenCalledWith(1, CONTAIN_1080P, true)
  })

  it("T-ui-autoplay-03 [Business rule] the stage stays aria-busy until player_open resolves; the first paused event does not make it ready by itself", async () => {
    // Arrange
    const t = await mountMpv({ open: "pending" })
    const stage = await screen.findByRole("img", { name: "v1.mxf" })
    // Act 1: 열기 중 paused 이벤트
    t.emitState({ state: "paused", timeMs: 0 })
    // Assert 1
    expect(stage).toHaveAttribute("aria-busy", "true")
    // Act 2
    await t.finishOpen()
    // Assert 2
    await t.ready()
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
  })

  it("T-ui-autoplay-04 [Business rule] changing the setting after the paused open does not reopen or restart the player", async () => {
    // Arrange
    const t = await mountMpv({ settings: { mediaOpenAutoplay: false } })
    await t.ready()
    // Act
    await t.api.updateSettings({ mediaOpenAutoplay: true })
    await act(async () => void (await t.queryClient.invalidateQueries({ queryKey: ["settings"] })))
    // Assert
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(t.open).toHaveBeenCalledWith(1, CONTAIN_1080P, false)
    expect(t.close).not.toHaveBeenCalled()
  })

  it("T-ui-autoplay-05 [Business rule] a seek-time open (AI transcript click) plays even when the setting is off", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: { mediaOpenAutoplay: false }, seekMs: 14_000 })
    await t.ready()
    // Assert
    expect(t.open).toHaveBeenCalledWith(1, CONTAIN_1080P, true)
    await waitFor(() => expect(t.seek).toHaveBeenCalledWith(14_000))
  })
})

describe("mouse input on the built-in player stage (DOM events)", () => {
  it("T-ui-input-01 [Happy] click toggles play/pause through the player control", async () => {
    // Arrange
    const t = await mountMpv()
    await t.ready()
    // Act
    fireEvent.click(t.stage())
    // Assert
    expect(t.control).toHaveBeenCalledWith("toggle")
  })

  it("T-ui-input-02 [Happy] wheel steps volume by 5% per notch (down lowers, up raises) and shows the level inside the control bar", async () => {
    // Arrange
    const t = await mountMpv()
    await t.ready()
    // Act 1
    fireEvent.wheel(t.stage(), { deltaY: 200 })
    // Assert 1
    expect(t.setVolume).toHaveBeenLastCalledWith(90, false)
    expect(screen.getByText("음량 90%")).toBeInTheDocument()
    // Act 2
    fireEvent.wheel(t.stage(), { deltaY: -100 })
    // Assert 2
    expect(t.setVolume).toHaveBeenLastCalledWith(95, false)
  })

  it("T-ui-input-03 [Boundary] fractional (high-resolution) wheel deltas accumulate to whole notches; a direction change or an idle gap drops the remainder; zero ignored", async () => {
    // Arrange
    const t = await mountMpv()
    await t.ready()
    t.setVolume.mockClear()
    vi.useFakeTimers({ toFake: ["Date"] })
    const wheel = (deltaY: number) => fireEvent.wheel(t.stage(), { deltaY })
    // Act 1: 0.5 + 0.5 → 한 노치
    wheel(50)
    expect(t.setVolume).not.toHaveBeenCalled()
    wheel(50)
    // Assert 1
    expect(t.setVolume).toHaveBeenCalledTimes(1)
    expect(t.setVolume).toHaveBeenLastCalledWith(95, false)
    // Act 2: 방향이 바뀌면 남은 값을 버린다 (-0.6 뒤 +0.6 → 0)
    wheel(60)
    wheel(-60)
    expect(t.setVolume).toHaveBeenCalledTimes(1)
    // Act 3: 남은 +0.6 은 오래 멈췄다 돌리면 더해지지 않는다 (더해지면 1.2 → 한 노치)
    vi.setSystemTime(Date.now() + WHEEL_IDLE_MS + 1)
    wheel(-60)
    expect(t.setVolume).toHaveBeenCalledTimes(1)
    // Act 4: 무효 값
    wheel(0)
    expect(t.setVolume).toHaveBeenCalledTimes(1)
  })

  it("T-ui-input-04 [Validation] a click before the open finished does nothing; a menu opened over the video changes neither the stage input nor the player (no snapshot mode)", async () => {
    // Arrange: 열기 중 클릭은 무시
    const t = await mountMpv({ open: "pending" })
    const stage = await screen.findByRole("img", { name: "v1.mxf" })
    fireEvent.click(stage)
    expect(t.control).not.toHaveBeenCalled()
    await t.finishOpen()
    await t.ready()
    // Act 1: 메뉴를 열어도 영상은 그대로 — 스냅샷·숨김 없음
    fireEvent.contextMenu(screen.getByTestId("player-track"))
    await screen.findByRole("menu")
    expect(t.canvas()).toBeInTheDocument()
    expect(t.setRenderActive).not.toHaveBeenCalledWith(false)
    // Act 2: 웹 stage 의 클릭·휠이 같은 동작을 한다
    const covered = screen.getByRole("img", { name: "v1.mxf", hidden: true })
    fireEvent.click(covered)
    fireEvent.wheel(covered, { deltaY: 100 })
    // Assert
    expect(t.control).toHaveBeenCalledWith("toggle")
    expect(t.setVolume).toHaveBeenLastCalledWith(95, false)
  })

  it("T-ui-input-05 [Happy] the arrow-key volume also flashes the level in the control bar", async () => {
    // Arrange
    const t = await mountMpv()
    await t.ready()
    // Act
    fireEvent.keyDown(document.body, { key: "ArrowDown", code: "ArrowDown" })
    // Assert
    expect(t.setVolume).toHaveBeenLastCalledWith(95, false)
    expect(screen.getByText("음량 95%")).toBeInTheDocument()
  })
})

describe("built-in player ranges-only (player_set_ranges)", () => {
  it("T-ui-ranges-30 [Business rule] toggle off at open sends nothing; turning the single-play toggle on sends the ranges, off again sends null", async () => {
    // Arrange
    const t = await mountMpv({ ranges: RANGES })
    await t.ready()
    await screen.findByRole("button", { name: /구간 1/ })
    expect(t.setPlayerRanges).not.toHaveBeenCalled()
    const toggle = screen.getByRole("button", { name: "구간만" })
    // Act 1: 켬
    fireEvent.click(toggle)
    // Assert 1
    await waitFor(() => expect(t.setPlayerRanges).toHaveBeenCalledWith(1, RANGES))
    await waitFor(() => expect(screen.getByRole("button", { name: "구간만" })).toHaveAttribute("aria-pressed", "true"))
    expect((await t.api.getSettings()).playRangesOnly).toBe(true)
    // Act 2: 끔
    fireEvent.click(screen.getByRole("button", { name: "구간만" }))
    // Assert 2
    await waitFor(() => expect(t.setPlayerRanges).toHaveBeenLastCalledWith(1, null))
    expect(t.setPlayerRanges).toHaveBeenCalledTimes(2)
  })

  it("T-ui-ranges-31 [Business rule] with the toggle already on, ranges are sent after the open succeeds (never before) and again when a range is edited", async () => {
    // Arrange: 열기가 끝나기 전에는 보내지 않는다
    // 이어보기 조회가 끝난 뒤에 열리므로 구간이 열기 전에 도착한다 — 이 시험의 전제(열기 뒤 도착)를 유지하려고 이어보기를 끈다
    const t = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: true, resumeEnabled: false }, open: "pending" })
    await screen.findByRole("img", { name: "v1.mxf" })
    await screen.findByRole("button", { name: /구간 1/ })
    expect(t.setPlayerRanges).not.toHaveBeenCalled()
    // Act 1: 열기 성공
    t.finishOpen()
    await t.ready()
    // Assert 1
    await waitFor(() => expect(t.setPlayerRanges).toHaveBeenCalledTimes(1))
    expect(t.setPlayerRanges).toHaveBeenLastCalledWith(1, RANGES)
    const order = [t.open.mock.invocationCallOrder[0], t.setPlayerRanges.mock.invocationCallOrder[0]]
    expect(order[0]).toBeLessThan(order[1])
    // Act 2: 구간 삭제(선택 후 Delete) → 남은 구간이 다시 간다
    await selectRangeByContextMenu(screen.getByRole("button", { name: /구간 2/ }))
    fakeTimers()
    fireEvent.keyDown(document.body, { code: "Delete", key: "Delete" })
    await advance(400)
    // Assert 2
    expect(t.setRanges).toHaveBeenCalledWith(1, [RANGES[0]])
    vi.useRealTimers()
    await waitFor(() => expect(t.setPlayerRanges).toHaveBeenLastCalledWith(1, [RANGES[0]]))
  })

  it("T-ui-ranges-32 [Business rule] D7: toggle on but the item has no ranges → nothing is sent (plays in full); the toggle and the count are hidden with 0 ranges", async () => {
    // Arrange + Act
    const t = await mountMpv({ ranges: [], settings: { playRangesOnly: true } })
    await t.ready()
    // Assert
    expect(screen.queryByRole("button", { name: "구간만" })).not.toBeInTheDocument()
    expect(screen.queryByText("구간 0개")).not.toBeInTheDocument()
    expect(t.setPlayerRanges).not.toHaveBeenCalled()
  })

  it("T-ui-ranges-33 [Business rule] collection item follows the collection toggle, not the single-play one; the control-bar toggle is hidden while a collection plays", async () => {
    // Arrange: 단독 켬 / 컬렉션 끔 → 연속 재생 항목에는 보내지 않는다
    const off = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: true, collectionPlayRangesOnly: false, resumeEnabled: false }, queue: [mpvQueueItem(1)] })
    await off.ready()
    await screen.findByRole("button", { name: /구간 1/ })
    // Assert 1
    expect(off.setPlayerRanges).not.toHaveBeenCalled()
    expect(screen.getAllByRole("button", { name: "구간만" })).toHaveLength(1) // 재생 바의 토글만
    off.view.unmount()
    // Arrange 2: 컬렉션 켬
    const on = await mountMpv({ ranges: RANGES, settings: { collectionPlayRangesOnly: true, resumeEnabled: false }, queue: [mpvQueueItem(1)] })
    await on.ready()
    // Assert 2
    await waitFor(() => expect(on.setPlayerRanges).toHaveBeenCalledWith(1, RANGES))
  })

  it("T-ui-ranges-34 [External] a failed player_set_ranges does not break playback or the UI", async () => {
    // Arrange
    // 이어보기 조회가 끝난 뒤에 열리므로 구간이 열기 전에 도착한다 — 이 시험의 전제(열기 뒤 도착)를 유지하려고 이어보기를 끈다
    const t = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: true, resumeEnabled: false } })
    t.setPlayerRanges.mockRejectedValue({ code: "internal", message: "x" })
    // Act
    await t.ready()
    await waitFor(() => expect(t.setPlayerRanges).toHaveBeenCalled())
    // Assert
    expect(screen.getByRole("img", { name: "v1.mxf" })).toHaveAttribute("aria-busy", "false")
    expect(screen.queryByRole("alert")).toBeNull()
  })
})
