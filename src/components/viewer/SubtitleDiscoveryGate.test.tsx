// Categories: Business rule (T-ui-sub-10 discovery only after a successful open) / External (T-ui-sub-11 failed open never discovers)
import { act, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { mountHtmlPlayer, restoreFrames } from "@/test/ranges"
import { stubTextTracks, subEntry, subTrack } from "@/test/subtitles"
import { mountMpv } from "@/test/mpv"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

let restoreTracks: () => void = () => undefined
beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  restoreTracks()
  restoreFrames()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const SUBS = { 1: [subEntry(subTrack("ko1", "ko"))] }

describe("built-in player: subtitles are discovered only after player_open succeeds", () => {
  it("T-ui-sub-10 [Business rule] not called while the open is pending; called exactly once after it succeeds; the CC button waits disabled meanwhile", async () => {
    // Arrange
    const t = await mountMpv({ open: "pending", subtitles: SUBS })
    const tracks = vi.spyOn(t.api, "subtitleTracks")
    await waitFor(() => expect(t.open).toHaveBeenCalled())
    await act(async () => void (await new Promise((r) => setTimeout(r, 20))))
    // Assert: 열리는 중
    expect(tracks).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: "자막" })).toBeDisabled()
    // Act: 열기 성공
    t.finishOpen()
    await t.ready()
    // Assert
    await waitFor(() => expect(t.setSubtitle).toHaveBeenCalledWith(1, "ko1"))
    expect(tracks).toHaveBeenCalledTimes(1)
    expect(screen.getByRole("button", { name: "자막" })).toBeEnabled()
  })

  it("T-ui-sub-10 [Business rule] a genuine re-open of the same asset discovers again", async () => {
    // Arrange
    const first = await mountMpv({ subtitles: SUBS })
    await first.ready()
    await waitFor(() => expect(first.setSubtitle).toHaveBeenCalled())
    first.view.unmount()
    // Act: 새 세션으로 같은 자산 열기
    const second = await mountMpv({ subtitles: SUBS })
    const secondCalls = vi.spyOn(second.api, "subtitleTracks")
    await second.ready()
    await waitFor(() => expect(second.setSubtitle).toHaveBeenCalledWith(1, "ko1"))
    // Assert
    expect(secondCalls).toHaveBeenCalledTimes(1)
  })

  it("T-ui-sub-11 [External] player_open rejecting → subtitle_tracks is never called", async () => {
    // Arrange
    const t = await mountMpv({ open: "fail", subtitles: SUBS })
    const tracks = vi.spyOn(t.api, "subtitleTracks")
    // Act
    await waitFor(() => expect(t.open).toHaveBeenCalled())
    expect(await screen.findByRole("alert")).toBeInTheDocument()
    await act(async () => void (await new Promise((r) => setTimeout(r, 20))))
    // Assert
    expect(tracks).not.toHaveBeenCalled()
    expect(t.setSubtitle).not.toHaveBeenCalled()
  })
})

describe("<video>: subtitles are discovered only after metadata is loaded", () => {
  it("T-ui-sub-10 [Business rule] not called before loadedmetadata; once after; remounting the same asset discovers again", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const calls: number[] = []
    const setup = (api: Parameters<NonNullable<Parameters<typeof mountHtmlPlayer>[0]>["setup"] & object>[0]) => {
      const orig = api.subtitleTracks.bind(api)
      vi.spyOn(api, "subtitleTracks").mockImplementation((id) => (calls.push(id), orig(id)))
    }
    const t = await mountHtmlPlayer({ subtitles: SUBS, metadata: false, setup })
    await act(async () => void (await new Promise((r) => setTimeout(r, 20))))
    // Assert: 메타데이터 전
    expect(calls).toEqual([])
    expect(screen.getByRole("button", { name: "자막" })).toBeDisabled()
    // Act
    act(() => void t.el.dispatchEvent(new Event("loadedmetadata")))
    // Assert
    await waitFor(() => expect(tt.tracks[0]?.mode).toBe("showing"))
    expect(calls).toEqual([1])
    t.view.unmount()
    // Act: 같은 자산을 다시 열기
    const again = await mountHtmlPlayer({ subtitles: SUBS, metadata: false, setup })
    act(() => void again.el.dispatchEvent(new Event("loadedmetadata")))
    // Assert
    await waitFor(() => expect(calls).toEqual([1, 1]))
  })

  it("T-ui-sub-11 [External] a media error → subtitle_tracks is never called, even if metadata events follow", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const calls: number[] = []
    const t = await mountHtmlPlayer({
      subtitles: SUBS,
      metadata: false,
      setup: (api) => void vi.spyOn(api, "subtitleTracks").mockImplementation(async (id) => (calls.push(id), [])),
    })
    // Act
    act(() => void t.el.dispatchEvent(new Event("error")))
    act(() => void t.el.dispatchEvent(new Event("loadedmetadata")))
    await act(async () => void (await new Promise((r) => setTimeout(r, 20))))
    // Assert
    expect(calls).toEqual([])
    expect(screen.getByRole("button", { name: "자막" })).toBeDisabled()
    expect(tt.addTextTrack).not.toHaveBeenCalled()
  })
})
