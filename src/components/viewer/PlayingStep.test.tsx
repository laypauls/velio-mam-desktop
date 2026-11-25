// Categories: Happy (T-ui-step-01, 03, 05) / Business rule (T-ui-step-02, 04, 06) / Boundary (T-ui-step-07, 08)
import { act, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { PlayerStatePayload } from "@/lib/ipc-contract-player"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { mountMpv } from "@/test/mpv"

const FWD = "앞으로 (정지 시 1프레임)"
const BACK = "뒤로 (정지 시 1프레임)"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** <video> 경로 — 영상을 열고 media 스텁을 돌려준다 */
async function setupHtml(settings: { playerPlayingStepSec?: number } = {}, duration = 100) {
  stubLayout()
  const media = stubMedia(duration)
  const api = createMockApi({ count: 5 })
  await useStubbedDetail(api, { mediaType: "video", fileName: "clip.mp4", fps: 25 })
  if (settings.playerPlayingStepSec !== undefined) await api.updateSettings(settings)
  const user = userEvent.setup()
  renderApp(api)
  await screen.findByRole("listbox", { name: "미디어 목록" })
  act(() => useViewStore.getState().openPreview(1))
  const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
  return { media, user, video }
}

describe("<video> step while playing", () => {
  it("T-ui-step-01 [Happy] playing: the buttons seek by the configured seconds and keep playing (no pause)", async () => {
    // Arrange
    const { media, user, video } = await setupHtml({ playerPlayingStepSec: 10 })
    video.currentTime = 30
    media.state.paused = false
    // Act
    await user.click(screen.getByRole("button", { name: FWD }))
    const forward = video.currentTime
    await user.click(screen.getByRole("button", { name: BACK }))
    // Assert
    expect(forward).toBe(40)
    expect(video.currentTime).toBe(30)
    expect(media.pause).not.toHaveBeenCalled()
    expect(media.state.paused).toBe(false)
  })

  it("T-ui-step-02 [Business rule] paused: still one frame (1/fps) and stays paused", async () => {
    // Arrange
    const { media, user, video } = await setupHtml({ playerPlayingStepSec: 10 })
    video.currentTime = 10
    // Act
    await user.click(screen.getByRole("button", { name: FWD }))
    // Assert
    expect(video.currentTime).toBeCloseTo(10.04, 6)
    expect(media.state.paused).toBe(true)
  })

  it("T-ui-step-03 [Happy] keyboard . and , take the same path: seconds while playing, frame while paused", async () => {
    // Arrange
    const { media, user, video } = await setupHtml() // 기본 5초
    video.currentTime = 20
    media.state.paused = false
    // Act + Assert: 재생 중
    await user.keyboard(".")
    expect(video.currentTime).toBe(25)
    await user.keyboard(",")
    expect(video.currentTime).toBe(20)
    expect(media.state.paused).toBe(false)
    // 정지 중
    media.state.paused = true
    await user.keyboard(".")
    expect(video.currentTime).toBeCloseTo(20.04, 6)
  })

  it("T-ui-step-07 [Boundary] playing: clamps to 0 and to the duration", async () => {
    // Arrange
    const { media, user, video } = await setupHtml({ playerPlayingStepSec: 30 }, 100)
    media.state.paused = false
    video.currentTime = 10
    // Act + Assert
    await user.click(screen.getByRole("button", { name: BACK }))
    expect(video.currentTime).toBe(0)
    video.currentTime = 90
    await user.click(screen.getByRole("button", { name: FWD }))
    expect(video.currentTime).toBe(100)
  })
})

/** 내장 플레이어 경로 — 재생 불가 영상 + 내장 플레이어 IPC 스파이 (길이 60초) */
async function setupMpv(settings: { playerPlayingStepSec?: number } = {}) {
  const t = await mountMpv({ settings, reply: { lengthMs: 60_000 }, detail: { durationMs: 60_000 } })
  await t.ready()
  return {
    control: t.control,
    seek: t.seek,
    emit: (p: Partial<PlayerStatePayload> & Pick<PlayerStatePayload, "state">) => t.emitState({ lengthMs: 60_000, ...p }),
  }
}

describe("built-in player step while playing", () => {
  it("T-ui-step-04 [Business rule] playing: buttons use the seek command (not stepForward/stepBack) with the configured seconds", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await setupMpv({ playerPlayingStepSec: 10 })
    await t.emit({ state: "playing", timeMs: 20_000 })
    // Act
    await user.click(screen.getByRole("button", { name: FWD }))
    // Assert
    expect(t.seek).toHaveBeenLastCalledWith(30_000)
    expect(t.control).not.toHaveBeenCalledWith("stepForward")
    await t.emit({ state: "playing", timeMs: 30_000 })
    await user.click(screen.getByRole("button", { name: BACK }))
    expect(t.seek).toHaveBeenLastCalledWith(20_000)
    expect(t.control).not.toHaveBeenCalledWith("stepBack")
  })

  it("T-ui-step-05 [Happy] playing: keyboard . and , seek by the default 5 seconds", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await setupMpv()
    await t.emit({ state: "playing", timeMs: 20_000 })
    // Act
    await user.keyboard(".")
    // Assert
    expect(t.seek).toHaveBeenLastCalledWith(25_000)
    expect(t.control).not.toHaveBeenCalled()
    await t.emit({ state: "playing", timeMs: 25_000 })
    await user.keyboard(",")
    expect(t.seek).toHaveBeenLastCalledWith(20_000)
  })

  it("T-ui-step-06 [Business rule] paused: frame commands (no fps argument) and no seek", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await setupMpv({ playerPlayingStepSec: 10 })
    await t.emit({ state: "paused", timeMs: 20_000 })
    // Act
    await user.click(screen.getByRole("button", { name: FWD }))
    await user.click(screen.getByRole("button", { name: BACK }))
    // Assert
    expect(t.control).toHaveBeenNthCalledWith(1, "stepForward")
    expect(t.control).toHaveBeenNthCalledWith(2, "stepBack")
    expect(t.seek).not.toHaveBeenCalled()
  })

  it("T-ui-step-08 [Boundary] playing near the start/end clamps the seek target to 0 and the length", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await setupMpv({ playerPlayingStepSec: 30 })
    await t.emit({ state: "playing", timeMs: 10_000 })
    // Act + Assert
    await user.click(screen.getByRole("button", { name: BACK }))
    expect(t.seek).toHaveBeenLastCalledWith(0)
    await t.emit({ state: "playing", timeMs: 50_000 })
    await user.click(screen.getByRole("button", { name: FWD }))
    expect(t.seek).toHaveBeenLastCalledWith(60_000)
  })
})
