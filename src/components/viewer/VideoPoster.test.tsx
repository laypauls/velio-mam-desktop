// T-ui-poster-01..17: 정지 상태로 여는 영상의 포스터(썸네일 + ▶) 게이트 — `<video>` 경로에만 적용된다.
// 내장 플레이어(libmpv)는 정지로 열어도 첫 프레임이 캔버스에 나오므로 포스터가 없다 (T-ui-poster-06..09, 11, 17 이 그 반대 방향을 검증한다).
// Categories: Business rule (-01, -02, -04..-09, -11) / Validation (-03 disabled controls, -10 key scoping) / Boundary (-12 thumbnail error) / Happy (-13 <video> start, -14 nav, -16 audio) / Business rule (-15 seek on <video>)
import { act, fireEvent, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { useViewerNavStore, useViewStore } from "@/stores"
import { CONTAIN_1080P, mountMpv, mpvQueueItem } from "@/test/mpv"
import { renderHtmlPlayer } from "@/test/ranges"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  useViewerNavStore.getState().setStepper(null)
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const PAUSED = { mediaOpenAutoplay: false }
const posterRoot = () => screen.findByTestId("video-poster")
/** 가운데 ▶ (컨트롤 바의 ▶ 보다 먼저 나온다) */
const playButton = () => screen.getAllByRole("button", { name: "재생" })[0]

/** `<video>` 경로(내장 플레이어 꺼짐) + 정지 설정 — 포스터가 먼저 뜬다 */
async function pausedHtml(over: Parameters<typeof renderHtmlPlayer>[0] = {}) {
  const t = await renderHtmlPlayer({ settings: PAUSED, ...over })
  t.media.state.swallowFirstPlay = false
  return t
}

describe("<video> opened paused shows a poster first", () => {
  it("T-ui-poster-01 [Business rule] no <video> before start; thumbnail + play button; ▶ creates it once and plays", async () => {
    // Arrange
    const t = await pausedHtml()
    // Assert (before)
    expect(await posterRoot()).toBeInTheDocument()
    expect(screen.getByTestId("poster-thumb")).toHaveAttribute("src")
    expect(document.querySelector("video")).toBeNull()
    // Act
    await userEvent.click(playButton())
    // Assert (after)
    await waitFor(() => expect(t.media.play).toHaveBeenCalledTimes(1))
    expect(document.querySelectorAll("video")).toHaveLength(1)
    expect(screen.queryByTestId("video-poster")).toBeNull()
  })

  it("T-ui-poster-02 [Business rule] clicking the stage (thumbnail) also starts (once)", async () => {
    // Arrange
    const t = await pausedHtml()
    const root = await posterRoot()
    // Act: 무대(썸네일) 클릭
    await userEvent.click(screen.getByTestId("poster-thumb"))
    // Assert
    await waitFor(() => expect(t.media.play).toHaveBeenCalledTimes(1))
    expect(root).not.toBeInTheDocument()
  })

  it("T-ui-poster-03 [Validation] disabled controls before start: seek/step/rate/mute are disabled and nothing is created", async () => {
    // Arrange
    const t = await pausedHtml()
    await posterRoot()
    // Act
    fireEvent.pointerDown(screen.getByTestId("player-track"), { clientX: 300, button: 0, pointerId: 1 })
    fireEvent.pointerUp(screen.getByTestId("player-track"), { clientX: 300, pointerId: 1 })
    await userEvent.click(screen.getByRole("button", { name: "뒤로 (정지 시 1프레임)" }))
    await userEvent.click(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" }))
    // Assert
    expect(screen.getByRole("button", { name: "뒤로 (정지 시 1프레임)" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" })).toBeDisabled()
    expect(screen.getByRole("combobox", { name: "재생 속도" })).toBeDisabled()
    expect(screen.getByRole("button", { name: "음소거" })).toBeDisabled()
    expect(screen.getByRole("slider", { name: "재생 위치" })).toHaveAttribute("aria-disabled", "true")
    expect(document.querySelector("video")).toBeNull()
    expect(t.media.play).not.toHaveBeenCalled()
    expect(screen.getByText("0:00 / 1:40")).toBeInTheDocument()
  })

  it("T-ui-poster-04 [Business rule] Space starts from anywhere; player shortcuts like ArrowRight do nothing before start", async () => {
    // Arrange
    const t = await pausedHtml()
    await posterRoot()
    // Act + Assert: 시작 전 다른 단축키는 무시
    fireEvent.keyDown(window, { key: "ArrowRight" })
    fireEvent.keyDown(window, { key: "." })
    expect(document.querySelector("video")).toBeNull()
    expect(t.media.play).not.toHaveBeenCalled()
    // Act: Space
    fireEvent.keyDown(window, { key: " " })
    // Assert
    await waitFor(() => expect(t.media.play).toHaveBeenCalledTimes(1))
  })

  it("T-ui-poster-05 [Business rule] Enter on the poster starts it", async () => {
    // Arrange
    const t = await pausedHtml()
    const root = await posterRoot()
    // Act
    fireEvent.keyDown(root, { key: "Enter" })
    // Assert
    await waitFor(() => expect(t.media.play).toHaveBeenCalledTimes(1))
  })

  it("T-ui-poster-10 [Validation] Enter outside the poster (gallery) and keys while a dialog is open do not start", async () => {
    // Arrange
    const t = await pausedHtml()
    await posterRoot()
    // Act: 갤러리 등 포스터 밖의 Enter
    fireEvent.keyDown(document.body, { key: "Enter" })
    // Act: 열린 대화상자 중 Space
    const dialog = document.createElement("div")
    dialog.setAttribute("role", "dialog")
    document.body.appendChild(dialog)
    fireEvent.keyDown(window, { key: " " })
    dialog.remove()
    // Assert
    await act(async () => void (await Promise.resolve()))
    expect(t.media.play).not.toHaveBeenCalled()
    expect(screen.getByTestId("video-poster")).toBeInTheDocument()
  })

  it("T-ui-poster-12 [Boundary] a thumbnail that fails to load leaves the background and the play button", async () => {
    // Arrange
    const t = await pausedHtml()
    await posterRoot()
    // Act
    fireEvent.error(screen.getByTestId("poster-thumb"))
    // Assert
    expect(screen.queryByTestId("poster-thumb")).toBeNull()
    expect(playButton()).toBeInTheDocument()
    expect(t.media.play).not.toHaveBeenCalled()
  })

  it("T-ui-poster-14 [Business rule] prev/next content stays functional on the poster", async () => {
    // Arrange
    const t = await pausedHtml()
    const step = vi.fn()
    act(() => useViewerNavStore.getState().setStepper({ canPrev: true, canNext: true, step }))
    await posterRoot()
    // Act
    await userEvent.click(screen.getByRole("button", { name: "다음 콘텐츠" }))
    // Assert
    expect(step).toHaveBeenCalledWith(1)
    expect(t.media.play).not.toHaveBeenCalled()
  })
})

describe("built-in player has no poster", () => {
  it("T-ui-poster-06 [Business rule] paused setting: the built-in player opens at once with autoplay=false and shows the first frame area, no poster", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: PAUSED })
    await t.ready()
    // Assert
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(t.open).toHaveBeenCalledWith(1, CONTAIN_1080P, false)
    expect(t.canvas()).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
  })

  it("T-ui-poster-07 [Business rule] collection playback current item opens with autoplay=true even with the paused setting", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: PAUSED, queue: [mpvQueueItem(1), mpvQueueItem(2)] })
    await t.ready()
    // Assert
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(t.open.mock.calls[0]?.[2]).toBe(true)
  })

  it("T-ui-poster-08 [Business rule] opened with a seek time: opens with autoplay and seeks there", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: PAUSED, seekMs: 5000 })
    await t.ready()
    // Assert
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(t.open.mock.calls[0]?.[2]).toBe(true)
    await waitFor(() => expect(t.seek).toHaveBeenCalledWith(5000))
  })

  it("T-ui-poster-09 [Business rule] a seek request arriving after the paused open seeks the same session — no reopen, no poster", async () => {
    // Arrange
    const t = await mountMpv({ settings: PAUSED })
    await t.ready()
    // Act
    act(() => useViewStore.getState().seekPreview(1, 7000))
    // Assert
    await waitFor(() => expect(t.seek).toHaveBeenCalledWith(7000))
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(screen.queryByTestId("video-poster")).toBeNull()
  })

  it("T-ui-poster-11 [Business rule] a video the app cannot play (no internal player, not WebView-playable) keeps the external-open notice, no poster", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: PAUSED, available: false, detail: { playable: false } })
    // Assert
    expect(await screen.findByText(/내장 플레이어를 사용할 수 없어/)).toBeInTheDocument()
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(t.open).not.toHaveBeenCalled()
  })

  it("T-ui-poster-17 [Concurrency] non-playable video: while player_available is pending nothing opens; once it resolves true the player opens paused, still without a poster", async () => {
    // Arrange: 가용성 조회가 늦게 끝난다
    let resolveAvailable: (v: boolean) => void = () => undefined
    const t = await mountMpv({ settings: PAUSED, detail: { playable: false }, setup: (api) => void vi.spyOn(api, "playerAvailable").mockImplementation(() => new Promise<boolean>((r) => (resolveAvailable = r))) })
    // Assert: 대기 중 — 로딩, 포스터·열기 없음
    await waitFor(() => expect(t.api.playerAvailable).toHaveBeenCalled())
    expect(screen.getByRole("status", { name: /불러오는 중/ })).toBeInTheDocument()
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(t.open).not.toHaveBeenCalled()
    // Act
    await act(async () => resolveAvailable(true))
    // Assert: 포스터 없이 정지로 열린다
    await t.ready()
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(t.open.mock.calls[0]?.[2]).toBe(false)
  })

  it("T-ui-poster-18 [Business rule] a WebView-playable video with the built-in player ON and available opens in the built-in player paused, no poster; with it unavailable the <video> poster shows", async () => {
    // Arrange + Act: 설정 ON + 사용 가능 → 포스터 없이 내장 플레이어
    const on = await mountMpv({ settings: PAUSED, detail: { playable: true } })
    await on.ready()
    // Assert
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(on.open.mock.calls[0]?.[2]).toBe(false)
    expect(document.querySelector("video")).toBeNull()
    on.view.unmount()
    // Arrange + Act: 설정 ON 이지만 플레이어 없음 → `<video>` 경로의 포스터
    const off = await mountMpv({ settings: PAUSED, detail: { playable: true }, available: false })
    // Assert
    expect(await posterRoot()).toBeInTheDocument()
    expect(off.open).not.toHaveBeenCalled()
  })
})

describe("<video> path (internal player off)", () => {
  it("T-ui-poster-13 [Business rule] paused setting: no <video> before start; start renders it and calls play()", async () => {
    // Arrange
    const t = await renderHtmlPlayer({ settings: PAUSED })
    t.media.state.swallowFirstPlay = false
    // Assert (before)
    expect(await posterRoot()).toBeInTheDocument()
    expect(document.querySelector("video")).toBeNull()
    expect(t.media.play).not.toHaveBeenCalled()
    // Act
    await userEvent.click(playButton())
    // Assert (after)
    await waitFor(() => expect(document.querySelector("video")).not.toBeNull())
    await waitFor(() => expect(t.media.play).toHaveBeenCalledTimes(1))
    expect(screen.queryByTestId("video-poster")).toBeNull()
  })

  it("T-ui-poster-15 [Business rule] <video> opened with a seek time skips the poster, plays and seeks", async () => {
    // Arrange + Act
    const t = await renderHtmlPlayer({ settings: PAUSED, seekMs: 4000 })
    t.media.state.swallowFirstPlay = false
    // Assert
    const el = await screen.findByLabelText<HTMLMediaElement>("clip.mp4", { selector: "video" })
    expect(screen.queryByTestId("video-poster")).toBeNull()
    await waitFor(() => expect(t.media.play).toHaveBeenCalled())
    expect(el.currentTime).toBe(4)
  })

  it("T-ui-poster-16 [Business rule] audio is never gated by the poster: with the paused setting it opens as an <audio> and does not play", async () => {
    // Arrange + Act
    const t = await renderHtmlPlayer({ settings: PAUSED, detail: { mediaType: "audio", fileName: "song.m4a", fps: null } })
    // Assert
    expect((await screen.findByLabelText<HTMLMediaElement>("song.m4a", { selector: "audio" })).tagName).toBe("AUDIO")
    expect(screen.queryByTestId("video-poster")).toBeNull()
    expect(t.media.play).not.toHaveBeenCalled()
  })
})
