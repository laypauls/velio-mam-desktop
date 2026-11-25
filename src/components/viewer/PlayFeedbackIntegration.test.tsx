// Categories: Happy (T-ui-feedback-10, 11, 15) / Business rule (T-ui-feedback-12, 13, 16, 17) / Boundary (T-ui-feedback-14, 18)
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

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const toggleEl = () => document.querySelector('[data-slot="play-feedback-toggle"]')
const seekEl = () => document.querySelector('[data-slot="play-feedback-seek"]')

/** <video> 경로 */
async function setupHtml(opts: { autoplay?: boolean } = {}) {
  stubLayout()
  const media = stubMedia(100)
  media.state.swallowFirstPlay = opts.autoplay !== true
  const api = createMockApi({ count: 5 })
  await useStubbedDetail(api, { mediaType: "video", fileName: "clip.mp4", fps: 25 })
  const user = userEvent.setup()
  renderApp(api)
  await screen.findByRole("listbox", { name: "미디어 목록" })
  act(() => useViewStore.getState().openPreview(1))
  const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
  return { media, user, video, stage: video.parentElement as HTMLElement }
}

describe("<video> center feedback", () => {
  it("T-ui-feedback-10 [Happy] stage click shows the state after the toggle (playing, then paused)", async () => {
    // Arrange
    const { user, stage } = await setupHtml()
    // Act
    await user.click(stage)
    // Assert
    expect(toggleEl()).toHaveAttribute("data-state", "playing")
    await user.click(stage)
    expect(toggleEl()).toHaveAttribute("data-state", "paused")
  })

  it("T-ui-feedback-11 [Happy] the control-bar play button and Space show it too (D1)", async () => {
    // Arrange
    const { user } = await setupHtml()
    // Act
    await user.click(screen.getByRole("button", { name: "재생" }))
    // Assert
    expect(toggleEl()).toHaveAttribute("data-state", "playing")
    await user.keyboard(" ")
    expect(toggleEl()).toHaveAttribute("data-state", "paused")
  })

  it("T-ui-feedback-12 [Business rule] playing: . shows +5 and accumulates; , flips direction and resets", async () => {
    // Arrange
    const { media, user, video } = await setupHtml()
    video.currentTime = 20
    media.state.paused = false
    // Act + Assert
    await user.keyboard("..")
    expect(seekEl()).toHaveAttribute("data-state", "forward")
    expect(screen.getByText("+10초")).toBeInTheDocument()
    await user.keyboard(",")
    expect(seekEl()).toHaveAttribute("data-state", "back")
    expect(screen.getByText("−5초")).toBeInTheDocument()
  })

  it("T-ui-feedback-13 [Business rule] paused: . (frame step) shows nothing", async () => {
    // Arrange
    const { user, video } = await setupHtml()
    video.currentTime = 20
    // Act
    await user.keyboard(".")
    // Assert
    expect(seekEl()).toBeNull()
    expect(toggleEl()).toBeNull()
  })

  it("T-ui-feedback-14 [Boundary] ArrowLeft at 0 and ArrowRight at the end show nothing", async () => {
    // Arrange
    const { user, video } = await setupHtml()
    video.currentTime = 0
    // Act
    await user.keyboard("{ArrowLeft}")
    video.currentTime = 100
    await user.keyboard("{ArrowRight}")
    // Assert
    expect(seekEl()).toBeNull()
  })

  it("T-ui-feedback-15 [Happy] ArrowRight shows +5 seconds", async () => {
    // Arrange
    const { user, video } = await setupHtml()
    video.currentTime = 20
    // Act
    await user.keyboard("{ArrowRight}")
    // Assert
    expect(screen.getByText("+5초")).toBeInTheDocument()
  })

  it("T-ui-feedback-16 [Business rule] autoplay on open shows nothing", async () => {
    // Arrange + Act
    const { media } = await setupHtml({ autoplay: true })
    // Assert
    expect(media.state.paused).toBe(false)
    expect(toggleEl()).toBeNull()
    expect(seekEl()).toBeNull()
  })
})

/** 내장 플레이어 경로 — 길이 60초 */
async function setupMpv() {
  const t = await mountMpv({ reply: { lengthMs: 60_000 }, detail: { durationMs: 60_000 } })
  await t.ready()
  return { ...t, emit: (p: Partial<PlayerStatePayload> & Pick<PlayerStatePayload, "state">) => t.emitState({ lengthMs: 60_000, ...p }) }
}

describe("built-in player center feedback", () => {
  it("T-ui-feedback-17 [Business rule] state events show nothing; stage click and playing . show the toggle and seek; paused frame step adds nothing", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await setupMpv()
    // Act + Assert: 상태 이벤트(자동 재생 시작)만으로는 표시하지 않는다
    await t.emit({ state: "playing", timeMs: 20_000 })
    expect(toggleEl()).toBeNull()
    expect(seekEl()).toBeNull()
    // 무대 클릭: 재생 중 → 일시정지 표시
    await user.click(t.stage())
    expect(toggleEl()).toHaveAttribute("data-state", "paused")
    // 재생 중 . → 앞으로 5초
    await user.keyboard(".")
    expect(screen.getByText("+5초")).toBeInTheDocument()
    // 정지 중 . → 1프레임 명령만 보내고 누적은 늘지 않는다
    await t.emit({ state: "paused", timeMs: 25_000 })
    await user.keyboard(".")
    expect(screen.getByText("+5초")).toBeInTheDocument()
    expect(t.control).toHaveBeenCalledWith("stepForward")
  })

  it("T-ui-feedback-18 [Boundary] ArrowLeft at 0 shows nothing; the control-bar play button shows the toggle", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await setupMpv()
    await t.emit({ state: "paused", timeMs: 0 })
    // Act
    await user.keyboard("{ArrowLeft}")
    // Assert
    expect(seekEl()).toBeNull()
    await user.click(screen.getByRole("button", { name: "재생" }))
    expect(toggleEl()).toHaveAttribute("data-state", "playing")
  })
})
