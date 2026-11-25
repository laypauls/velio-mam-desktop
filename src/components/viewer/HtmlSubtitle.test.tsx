// Categories: Happy (T-ui-sub-02, 03) / Validation (T-ui-sub-04) / Business rule (T-ui-sub-07) / External (T-ui-sub-02 offline, T-ui-sub-08 cues failure)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { mountHtmlPlayer, restoreFrames } from "@/test/ranges"
import { stubTextTracks, subEntry, subTrack } from "@/test/subtitles"
import { escapeCueText } from "./useHtmlSubtitle"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

let restoreTracks: () => void = () => undefined
beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  vi.mocked(toast.error).mockClear()
})
afterEach(() => {
  restoreTracks()
  restoreFrames()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const KO = subTrack("영상.ko.srt", "ko")
const EN = subTrack("영상.en.srt", "en")
const SUBS = {
  1: [
    subEntry(KO, [
      { startMs: 0, endMs: 1000, text: "하나" },
      { startMs: 1000, endMs: 2000, text: "둘" },
      { startMs: 2000, endMs: 3000, text: "셋" },
    ]),
    subEntry(EN, [
      { startMs: 0, endMs: 1000, text: "one" },
      { startMs: 1000, endMs: 2000, text: "two" },
    ]),
  ],
}

const ccButton = () => screen.findByRole("button", { name: "자막" })

describe("<video> external subtitles", () => {
  it("T-ui-sub-02 [Happy] no tracks → the CC button is disabled with a 자막 없음 tooltip; audio has no CC button", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const user = userEvent.setup()
    const t = await mountHtmlPlayer()
    // Act
    const cc = await ccButton()
    await user.hover(cc.parentElement as HTMLElement)
    // Assert
    expect(cc).toBeDisabled()
    expect(cc).toHaveAttribute("aria-pressed", "false")
    expect((await screen.findAllByText("자막 없음")).length).toBeGreaterThan(0)
    expect(tt.addTextTrack).not.toHaveBeenCalled()
    t.view.unmount()
    // Arrange + Act: 음성
    await mountHtmlPlayer({ detail: { mediaType: "audio", fileName: "song.m4a", fps: null }, subtitles: SUBS })
    // Assert
    expect(screen.queryByRole("button", { name: "자막" })).not.toBeInTheDocument()
  })

  it("T-ui-sub-02 [External] asset_offline from subtitle_tracks → disabled CC button, no toast", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const t = await mountHtmlPlayer({
      setup: (api) => void vi.spyOn(api, "subtitleTracks").mockRejectedValue({ code: "asset_offline", message: "x" }),
    })
    // Act
    const cc = await ccButton()
    await waitFor(() => expect(t.api.subtitleTracks).toHaveBeenCalled())
    // Assert
    expect(cc).toBeDisabled()
    expect(toast.error).not.toHaveBeenCalled()
    expect(tt.addTextTrack).not.toHaveBeenCalled()
  })

  it("T-ui-sub-03 [Happy] default language is picked → one showing text track with all cues; switching reuses the track; 끄기 disables it and saves subtitleVisible false", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const user = userEvent.setup()
    const t = await mountHtmlPlayer({ subtitles: SUBS })
    const update = vi.spyOn(t.api, "updateSettings")
    // Assert: 자동 선택(ko)
    await waitFor(() => expect(tt.tracks[0]?.mode).toBe("showing"))
    expect(tt.tracks).toHaveLength(1)
    expect(tt.tracks[0].owner).toBe(t.el)
    expect(tt.tracks[0].cues.map((c) => c.text)).toEqual(["하나", "둘", "셋"])
    expect(tt.tracks[0].cues[1]).toMatchObject({ startTime: 1, endTime: 2 })
    // Act: 영어로 전환
    await user.click(await ccButton())
    const menu = await screen.findByRole("menu")
    expect(within(menu).getByRole("menuitemradio", { name: "한국어 (SRT)" })).toBeChecked()
    await user.click(within(menu).getByRole("menuitemradio", { name: "영어 (SRT)" }))
    // Assert: 같은 트랙 재사용, 큐 교체
    await waitFor(() => expect(tt.tracks[0].cues.map((c) => c.text)).toEqual(["one", "two"]))
    expect(tt.tracks).toHaveLength(1)
    expect(tt.addTextTrack).toHaveBeenCalledTimes(1)
    expect(tt.tracks[0].mode).toBe("showing")
    expect(update).not.toHaveBeenCalled() // 이미 표시 중이면 설정을 건드리지 않는다
    // Act: 끄기
    await user.click(await ccButton())
    await user.click(await screen.findByRole("menuitemradio", { name: "자막 끄기" }))
    // Assert
    await waitFor(() => expect(tt.tracks[0].mode).toBe("disabled"))
    expect(tt.tracks[0].cues).toHaveLength(0)
    await waitFor(() => expect(update).toHaveBeenCalledWith({ subtitleVisible: false }))
    expect((await t.api.getSettings()).subtitleVisible).toBe(false)
    expect(screen.getByRole("button", { name: "자막" })).toHaveAttribute("aria-pressed", "false")
    // Act: 꺼진 상태에서 트랙을 고르면 다시 켜진다
    await user.click(await ccButton())
    await user.click(await screen.findByRole("menuitemradio", { name: "한국어 (SRT)" }))
    // Assert
    await waitFor(() => expect(tt.tracks[0].mode).toBe("showing"))
    expect(tt.tracks[0].cues).toHaveLength(3)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ subtitleVisible: true }))
    expect(tt.addTextTrack).toHaveBeenCalledTimes(1)
  })

  it("T-ui-sub-03 [Business rule] subtitleVisible false at open → no cues shown until a track is picked", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const user = userEvent.setup()
    await mountHtmlPlayer({ subtitles: SUBS, settings: { subtitleVisible: false } })
    // Act
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toBeEnabled()) // 찾는 중 버튼이 메뉴 버튼으로 교체된다
    const cc = screen.getByRole("button", { name: "자막" })
    // Assert
    expect(cc).toHaveAttribute("aria-pressed", "false")
    expect(tt.addTextTrack).not.toHaveBeenCalled()
    await user.click(cc)
    expect(await screen.findByRole("menuitemradio", { name: "자막 끄기" })).toBeChecked()
  })

  it("T-ui-sub-04 [Validation] cue text with < and & is escaped so it renders literally; plain text is unchanged", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const raw = "<b>굵게</b> & 1 < 2\n둘째 줄"
    await mountHtmlPlayer({ subtitles: { 1: [subEntry(KO, [{ startMs: 0, endMs: 1000, text: raw }])] } })
    // Act + Assert
    await waitFor(() => expect(tt.tracks[0]?.cues).toHaveLength(1))
    expect(tt.tracks[0].cues[0].text).toBe("&lt;b>굵게&lt;/b> &amp; 1 &lt; 2\n둘째 줄")
    expect(escapeCueText("&lt;")).toBe("&amp;lt;") // 이미 이스케이프된 것처럼 보이는 원문도 그대로 보이게
    expect(escapeCueText("안녕")).toBe("안녕")
  })

  it("T-ui-sub-07 [Business rule] an open CC menu is an OPEN_LAYER: Space and arrows are ignored; after closing, Space toggles play", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const user = userEvent.setup()
    const t = await mountHtmlPlayer({ subtitles: SUBS })
    await waitFor(() => expect(tt.tracks[0]?.mode).toBe("showing"))
    t.el.currentTime = 20
    // Act: 메뉴 열기 후 창 수준 키
    await user.click(await ccButton())
    await screen.findByRole("menu")
    act(() => {
      fireEvent.keyDown(document.body, { key: " ", code: "Space" })
      fireEvent.keyDown(document.body, { key: "ArrowRight", code: "ArrowRight" })
    })
    // Assert
    expect(t.media.play).not.toHaveBeenCalled()
    expect(t.el.currentTime).toBe(20)
    // Act: 닫은 직후 Space
    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument())
    await user.keyboard(" ")
    // Assert
    expect(t.media.play).toHaveBeenCalledTimes(1)
  })

  it("T-ui-sub-08 [External] subtitle_cues failing → one toast, playback UI intact, no text track shown", async () => {
    // Arrange
    const tt = stubTextTracks()
    restoreTracks = tt.restore
    const t = await mountHtmlPlayer({
      subtitles: SUBS,
      setup: (api) => void vi.spyOn(api, "subtitleCues").mockRejectedValue({ code: "not_found", message: "x" }),
    })
    // Act
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("자막을 불러오지 못했습니다."))
    // Assert
    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(t.el.isConnected).toBe(true)
    expect(screen.getByRole("button", { name: "재생" })).toBeEnabled()
    expect(tt.tracks.every((x) => x.mode !== "showing")).toBe(true)
  })
})
