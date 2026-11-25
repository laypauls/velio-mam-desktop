// Categories: Business rule (T-ui-autoplay-10..12 HTML) / External (T-ui-autoplay-14 NotAllowedError)
import { act, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { qk } from "@/hooks/keys"
import { usePlaybackStore } from "@/stores"
import { mountHtmlPlayer, queueItem, renderHtmlPlayer, restoreFrames } from "@/test/ranges"

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

const queue = [queueItem(1), queueItem(2)]

describe("<video>/<audio> open follows the autoplay setting (D11)", () => {
  it("T-ui-autoplay-10 [Business rule] single open: off (default) → video shows the poster and never plays on its own; on → play() once, for video and for audio", async () => {
    // Arrange + Act: 꺼짐 — 영상은 포스터(플레이어 미생성)
    const off = await renderHtmlPlayer({ settings: { mediaOpenAutoplay: false } })
    // Assert
    expect(await screen.findByTestId("video-poster")).toBeInTheDocument()
    expect(document.querySelector("video")).toBeNull()
    expect(off.media.play).not.toHaveBeenCalled()
    off.view.unmount()
    // Arrange + Act: 꺼짐 — 음성은 기존 그대로 (요소가 열리고 재생하지 않는다)
    const audioOff = await renderHtmlPlayer({ settings: { mediaOpenAutoplay: false }, detail: { mediaType: "audio", fileName: "song.m4a", fps: null } })
    await screen.findByLabelText("song.m4a", { selector: "audio" })
    expect(audioOff.media.play).not.toHaveBeenCalled()
    expect(screen.queryByTestId("video-poster")).toBeNull()
    audioOff.view.unmount()
    vi.restoreAllMocks()
    // Arrange + Act: 켬 — 영상
    const video = await mountHtmlPlayer({ settings: { mediaOpenAutoplay: true } })
    // Assert
    await waitFor(() => expect(video.media.play).toHaveBeenCalledTimes(1))
    video.view.unmount()
    vi.restoreAllMocks()
    // Arrange + Act: 켬 — 음성
    const audio = await mountHtmlPlayer({ settings: { mediaOpenAutoplay: true }, detail: { mediaType: "audio", fileName: "song.m4a", fps: null } })
    // Assert
    await waitFor(() => expect(audio.media.play).toHaveBeenCalledTimes(1))
    expect(audio.el.tagName).toBe("AUDIO")
  })

  it("T-ui-autoplay-11 [Business rule] collection playback always autoplays (no poster), with the setting off", async () => {
    // Arrange
    const t = await renderHtmlPlayer({ settings: { mediaOpenAutoplay: false } })
    t.media.state.swallowFirstPlay = false
    expect(await screen.findByTestId("video-poster")).toBeInTheDocument()
    expect(t.media.play).not.toHaveBeenCalled()
    // Act
    act(() => usePlaybackStore.getState().start(1, queue))
    // Assert
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    expect(screen.queryByTestId("video-poster")).toBeNull()
  })

  it("T-ui-autoplay-12 [Business rule] changing the setting while an item is open does not start it", async () => {
    // Arrange
    const t = await renderHtmlPlayer({ settings: { mediaOpenAutoplay: false } })
    expect(await screen.findByTestId("video-poster")).toBeInTheDocument()
    // Act
    await t.api.updateSettings({ mediaOpenAutoplay: true })
    await act(async () => {
      await t.queryClient.invalidateQueries({ queryKey: qk.settings })
      await new Promise((r) => setTimeout(r, 0))
    })
    // Assert
    expect(t.media.play).not.toHaveBeenCalled()
    expect(screen.getByTestId("video-poster")).toBeInTheDocument()
  })

  it("T-ui-autoplay-14 [External] a blocked autoplay (NotAllowedError) leaves the item paused: no error UI, no skip in a collection", async () => {
    // Arrange: 연속 재생 — 차단돼도 건너뛰지 않는다
    const t = await mountHtmlPlayer()
    t.media.state.swallowFirstPlay = false
    t.media.play.mockRejectedValue(new DOMException("blocked", "NotAllowedError"))
    // Act
    act(() => usePlaybackStore.getState().start(1, queue))
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    await act(async () => void (await Promise.resolve()))
    // Assert
    expect(usePlaybackStore.getState().currentId).toBe(1)
    expect(usePlaybackStore.getState().failed.size).toBe(0)
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
    expect(screen.queryByRole("alert")).toBeNull()
  })
})
