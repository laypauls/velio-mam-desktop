// Categories: Happy (T-ui-gensub-09, 12) / Business rule (T-ui-gensub-10, 11, 13) / External (T-ui-gensub-14 offline analysis result missing)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { qk } from "@/hooks/keys"
import { aiDetail, SEGMENTS, seedTranscript } from "@/test/generatedSubtitle"
import { mountMpv } from "@/test/mpv"
import { mountHtmlPlayer, restoreFrames } from "@/test/ranges"
import { stubTextTracks, subEntry, subTrack } from "@/test/subtitles"

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

const KO = subTrack("영상.ko.srt", "ko")
const FILE_SUBS = { 1: [subEntry(KO, [{ startMs: 0, endMs: 10_000, text: "파일 자막" }])] }
const cc = () => screen.findByRole("button", { name: "자막" })
const slot = () => document.querySelector("[data-slot=generated-subtitle]")

/** `<video>` 의 현재 시각을 정하고 탐색 완료 이벤트를 보낸다 */
function seekTo(el: HTMLMediaElement, sec: number) {
  el.currentTime = sec
  act(() => void el.dispatchEvent(new Event("seeked")))
}

describe("<video> generated subtitles", () => {
  it("T-ui-gensub-09 [Happy] file + generated: generated text sits on top, follows seeks, and is empty in gaps; CC is pressed", async () => {
    // Arrange
    restoreTracks = stubTextTracks().restore
    const t = await mountHtmlPlayer({ subtitles: FILE_SUBS, setup: (api) => void seedTranscript(api, SEGMENTS) })
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toHaveAttribute("aria-pressed", "true"))
    // Act + Assert: 구간 안
    seekTo(t.el, 1.5)
    expect(await screen.findByText("생성 하나")).toBeInTheDocument()
    expect(slot()).toHaveAttribute("data-position", "top")
    // Act: 구간 사이
    seekTo(t.el, 2.5)
    expect(screen.queryByText("생성 하나")).not.toBeInTheDocument()
    expect(slot()).toBeNull()
    // Act: 다음 구간 시작 시각 (포함)
    seekTo(t.el, 3)
    expect(screen.getByText("생성 둘")).toBeInTheDocument()
  })

  it("T-ui-gensub-10 [Business rule] generated only (no file tracks): bottom position, CC enabled and pressed, file group shows 자막 파일 없음", async () => {
    // Arrange
    restoreTracks = stubTextTracks().restore
    const user = userEvent.setup()
    const t = await mountHtmlPlayer({ setup: (api) => void seedTranscript(api, SEGMENTS) })
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toBeEnabled())
    // Act
    seekTo(t.el, 1.2)
    // Assert
    expect(await screen.findByText("생성 하나")).toBeInTheDocument()
    expect(slot()).toHaveAttribute("data-position", "bottom")
    expect(screen.getByRole("button", { name: "자막" })).toHaveAttribute("aria-pressed", "true")
    await user.click(screen.getByRole("button", { name: "자막" }))
    expect(await screen.findByRole("menuitem", { name: "자막 파일 없음" })).toBeInTheDocument()
  })

  it("T-ui-gensub-11 [Business rule] unchecking saves generatedSubtitleVisible false and removes the overlay; checking again restores it (gate ON/OFF)", async () => {
    // Arrange
    restoreTracks = stubTextTracks().restore
    const user = userEvent.setup()
    const t = await mountHtmlPlayer({ subtitles: FILE_SUBS, setup: (api) => void seedTranscript(api, SEGMENTS) })
    const update = vi.spyOn(t.api, "updateSettings")
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toHaveAttribute("aria-pressed", "true"))
    seekTo(t.el, 1.5)
    await screen.findByText("생성 하나")
    // Act: 끄기
    await user.click(screen.getByRole("button", { name: "자막" }))
    const box = await screen.findByRole("menuitemcheckbox", { name: "한국어 (자동 생성)" })
    await user.click(box)
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ generatedSubtitleVisible: false }))
    expect((await t.api.getSettings()).generatedSubtitleVisible).toBe(false)
    await waitFor(() => expect(screen.queryByText("생성 하나")).not.toBeInTheDocument())
    expect(update).not.toHaveBeenCalledWith(expect.objectContaining({ subtitleVisible: expect.anything() })) // 자막 파일 설정은 건드리지 않는다
    // 메뉴는 열려 있다 — 체크 해제 상태, 파일 자막은 그대로 표시 중
    await waitFor(() => expect(screen.getByRole("menuitemcheckbox", { name: "한국어 (자동 생성)" })).not.toBeChecked())
    expect(screen.getByRole("button", { name: "자막", hidden: true })).toHaveAttribute("aria-pressed", "true")
    // Act: 다시 켜기
    await user.click(screen.getByRole("menuitemcheckbox", { name: "한국어 (자동 생성)" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ generatedSubtitleVisible: true }))
    expect(await screen.findByText("생성 하나")).toBeInTheDocument()
  })

  it("T-ui-gensub-11 [Business rule] generatedSubtitleVisible false at open → nothing drawn and CC not pressed when no file subtitle is showing", async () => {
    // Arrange
    restoreTracks = stubTextTracks().restore
    const t = await mountHtmlPlayer({ settings: { generatedSubtitleVisible: false }, setup: (api) => void seedTranscript(api, SEGMENTS) })
    // Act
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toBeEnabled())
    seekTo(t.el, 1.5)
    // Assert
    expect(screen.getByRole("button", { name: "자막" })).toHaveAttribute("aria-pressed", "false")
    expect(screen.queryByText("생성 하나")).not.toBeInTheDocument()
  })

  it("T-ui-gensub-12 [Happy] while playing, currentTime is read each animation frame; pausing stops the loop (rAF stubbed with cancel)", async () => {
    // Arrange
    restoreTracks = stubTextTracks().restore
    const t = await mountHtmlPlayer({ setup: (api) => void seedTranscript(api, SEGMENTS) })
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toBeEnabled())
    await screen.findByRole("button", { name: "자막" })
    // Act: 재생 시작 (정지 상태에서 시작 → 루프 없음)
    act(() => t.frames.tick())
    t.el.currentTime = 1.1
    t.media.state.paused = false
    act(() => void t.el.dispatchEvent(new Event("play")))
    // Assert: 첫 sync 가 바로 읽는다
    expect(await screen.findByText("생성 하나")).toBeInTheDocument()
    // Act: 프레임 진행
    t.el.currentTime = 3.2
    act(() => t.frames.tick())
    expect(screen.getByText("생성 둘")).toBeInTheDocument()
    // Act: 일시정지 → 다음 프레임 뒤 루프 종료 (예약 없음)
    t.media.state.paused = true
    act(() => void t.el.dispatchEvent(new Event("pause")))
    act(() => t.frames.tick())
    const pending = t.frames.pendingRaf()
    t.el.currentTime = 1.1
    act(() => t.frames.tick())
    // Assert: 정지 중에는 currentTime 변화를 폴링하지 않는다 (seeked/pause 이벤트로만)
    expect(screen.getByText("생성 둘")).toBeInTheDocument()
    expect(t.frames.pendingRaf()).toBeLessThanOrEqual(pending)
  })

  it("T-ui-gensub-13 [Business rule] a refreshed analysis result (cache update) replaces the segments while open", async () => {
    // Arrange
    restoreTracks = stubTextTracks().restore
    const t = await mountHtmlPlayer({ setup: (api) => void seedTranscript(api, SEGMENTS) })
    seekTo(t.el, 1.5)
    await screen.findByText("생성 하나")
    // Act: 다시 생성 → 같은 구간 위치에 다른 문장
    act(() => t.queryClient.setQueryData(qk.assetAi(1), aiDetail([{ startMs: 1000, endMs: 2000, text: "새로 생성" }])))
    // Assert
    expect(await screen.findByText("새로 생성")).toBeInTheDocument()
    expect(screen.queryByText("생성 하나")).not.toBeInTheDocument()
    // Act: 자막이 모두 사라진 결과 (transcript null)
    act(() => t.queryClient.setQueryData(qk.assetAi(1), aiDetail(null)))
    // Assert
    await waitFor(() => expect(screen.queryByText("새로 생성")).not.toBeInTheDocument())
  })

  it("T-ui-gensub-14 [Business rule] audio never queries or draws generated subtitles and has no CC button; video before metadata does not query", async () => {
    // Arrange: 음성
    const a = await mountHtmlPlayer({ detail: { mediaType: "audio", fileName: "song.m4a", fps: null }, setup: (api) => void seedTranscript(api, SEGMENTS) })
    // Assert
    expect(a.api.getAssetAi).not.toHaveBeenCalled()
    expect(screen.queryByRole("button", { name: "자막" })).not.toBeInTheDocument()
    expect(slot()).toBeNull()
    a.view.unmount()
    // Arrange: 영상이지만 메타데이터 로드 전 (열기 성공 전)
    const v = await mountHtmlPlayer({ metadata: false, setup: (api) => void seedTranscript(api, SEGMENTS) })
    // Assert
    expect(v.api.getAssetAi).not.toHaveBeenCalled()
    expect(screen.getByRole("button", { name: "자막" })).toBeDisabled()
  })

  it("T-ui-gensub-14 [External] get_asset_ai failing → no overlay, CC stays disabled, player intact", async () => {
    // Arrange
    restoreTracks = stubTextTracks().restore
    const t = await mountHtmlPlayer({ setup: (api) => void vi.spyOn(api, "getAssetAi").mockRejectedValue({ code: "internal", message: "x" }) })
    // Act
    await waitFor(() => expect(t.api.getAssetAi).toHaveBeenCalled())
    seekTo(t.el, 1.5)
    // Assert
    expect(slot()).toBeNull()
    expect(screen.getByRole("button", { name: "자막" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "재생" })).toBeEnabled()
  })
})

describe("built-in (libmpv) player generated subtitles", () => {
  it("T-ui-gensub-15 [Happy] state events drive the overlay; file subtitle on → top, and the file track is still sent to the player", async () => {
    // Arrange
    const t = await mountMpv({ subtitles: { 1: [subEntry(subTrack("ko1", "ko")), subEntry(subTrack("en1", "en"))] }, setup: (api) => void seedTranscript(api, SEGMENTS) })
    await t.ready()
    await waitFor(() => expect(t.setSubtitle).toHaveBeenCalledWith(1, "ko1"))
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toHaveAttribute("aria-pressed", "true"))
    // Act
    t.emitState({ state: "paused", timeMs: 1500 })
    // Assert
    expect(await screen.findByText("생성 하나")).toBeInTheDocument()
    expect(slot()).toHaveAttribute("data-position", "top")
    expect(t.stage()).toContainElement(slot() as HTMLElement)
    // Act: 구간 사이
    t.emitState({ state: "paused", timeMs: 2500 })
    // Assert
    expect(screen.queryByText("생성 하나")).not.toBeInTheDocument()
  })

  it("T-ui-gensub-15 [Business rule] generated only → bottom; playing interpolates between the 5Hz events using a deterministic rAF stub", async () => {
    // Arrange
    let now = 50_000
    vi.spyOn(performance, "now").mockImplementation(() => now)
    const t = await mountMpv({ setup: (api) => void seedTranscript(api, SEGMENTS) })
    await t.ready()
    await waitFor(() => expect(screen.getByRole("button", { name: "자막" })).toBeEnabled())
    // Act: 재생 중 1900ms 위치 이벤트 → 아직 "생성 하나"
    t.emitState({ state: "playing", timeMs: 1900 })
    expect(await screen.findByText("생성 하나")).toBeInTheDocument()
    expect(slot()).toHaveAttribute("data-position", "bottom")
    // Act: 이벤트 없이 150ms 경과 → 2050ms (구간 끝 지남)
    now += 150
    await t.flushRaf()
    // Assert
    expect(screen.queryByText("생성 하나")).not.toBeInTheDocument()
    // Act: 정지 → 보간하지 않는다
    t.emitState({ state: "paused", timeMs: 3000 })
    expect(screen.getByText("생성 둘")).toBeInTheDocument()
    now += 5000
    await t.flushRaf()
    expect(screen.getByText("생성 둘")).toBeInTheDocument()
  })

  it("T-ui-gensub-16 [Business rule] toggling off in the menu saves the setting and removes the overlay; no generated subtitle → disabled CC (no file either)", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv({ setup: (api) => void seedTranscript(api, SEGMENTS) })
    const update = vi.spyOn(t.api, "updateSettings")
    await t.ready()
    t.emitState({ state: "paused", timeMs: 1500 })
    await screen.findByText("생성 하나")
    // Act
    await user.click(await cc())
    await user.click(await screen.findByRole("menuitemcheckbox", { name: "한국어 (자동 생성)" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ generatedSubtitleVisible: false }))
    await waitFor(() => expect(screen.queryByText("생성 하나")).not.toBeInTheDocument())
    expect(within(await screen.findByRole("menu")).getByText("생성된 자막")).toBeInTheDocument()
  })

  it("T-ui-gensub-16 [Business rule] no transcript and no file subtitle → CC disabled, nothing drawn", async () => {
    // Arrange
    const t = await mountMpv({ setup: (api) => void seedTranscript(api, null) })
    // Act
    await t.ready()
    t.emitState({ state: "paused", timeMs: 1500 })
    // Assert
    expect(await screen.findByRole("button", { name: "자막" })).toBeDisabled()
    expect(slot()).toBeNull()
  })
})
