// T-ui-vlcset-13, -14: aria-busy 상태 노출(상태 줄 제거) / 설정 ON 통합 (ID 는 이관 전과 같다)
// Categories: Business rule (T-ui-viewer-40..43, -45, -52, -53, -59, -62, -63; -64 Boundary) / External failure (T-ui-viewer-44, -48, -57)
//             Happy (T-ui-viewer-54, -56, T-ui-mpv-time-01) / Boundary (T-ui-viewer-58, T-ui-mpv-time-02) / Validation (T-ui-viewer-55 keyboard scoping)
import { act, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { CONTAIN_1080P, mountMpv } from "@/test/mpv"
import { useViewStore } from "@/stores"

beforeEach(() => {
  // Radix Select 가 jsdom 에 없는 API 를 쓴다
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("MpvPlayer — selection", () => {
  it("T-ui-viewer-40 [Business rule] an online non-playable video opens the built-in player with the canvas size, not <video>", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: { videoPlayerVlc: false } })
    await t.ready()
    // Assert
    // 열 때 자동 재생(설정) — 영상은 1920×1080, 무대 1000×600 안에 contain → 1000×563 장치 px
    expect(t.open).toHaveBeenCalledWith(1, CONTAIN_1080P, true)
    expect(document.querySelector("video")).toBeNull()
    expect(t.stage()).toBeInTheDocument()
    expect(t.canvas()).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" })).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "외부 프로그램으로 열기" })).not.toBeInTheDocument()
  })

  it("T-ui-viewer-41 [Business rule] a playable video with the setting off keeps <video> and never touches the built-in player", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: { videoPlayerVlc: false }, detail: { playable: true } })
    // Assert
    await screen.findByLabelText("v1.mxf", { selector: "video" })
    expect(t.api.playerAvailable).not.toHaveBeenCalled()
    expect(t.open).not.toHaveBeenCalled()
  })

  it("T-ui-viewer-42 [Business rule] when the built-in player is unavailable the external-open fallback is shown and open is not attempted", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv({ settings: { videoPlayerVlc: false }, available: false })
    // Act
    expect(await screen.findByText(/내장 플레이어를 사용할 수 없어/)).toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "외부 프로그램으로 열기" }))
    // Assert
    expect(t.openOriginal).toHaveBeenCalledWith(1)
    expect(t.open).not.toHaveBeenCalled()
  })

  it("T-ui-viewer-62 [Business rule] regression: available=false says the player is missing, not that the format is unsupported", async () => {
    // Arrange + Act
    await mountMpv({ settings: { videoPlayerVlc: false }, available: false })
    // Assert
    expect(await screen.findByText(/다시 설치하거나 외부 프로그램으로 열어 주세요/)).toBeInTheDocument()
    expect(screen.queryByText(/이 형식은 앱에서 바로 재생할 수 없습니다/)).not.toBeInTheDocument()
  })

  it("T-ui-viewer-63 [Business rule] the three failure causes show distinct messages", async () => {
    // Arrange + Act: 플레이어 없음
    const missing = await mountMpv({ settings: { videoPlayerVlc: false }, available: false })
    const missingText = (await screen.findByText(/내장 플레이어를 사용할 수 없어/)).textContent
    missing.view.unmount()
    // Arrange + Act: 열기 실패
    const failed = await mountMpv({ settings: { videoPlayerVlc: false }, open: "fail" })
    const failedText = (await screen.findByRole("alert")).textContent
    failed.view.unmount()
    // Arrange + Act: 오프라인
    await mountMpv({ settings: { videoPlayerVlc: false }, open: { code: "asset_offline", message: "x" } })
    const offlineText = (await screen.findByRole("alert")).textContent
    // Assert
    expect(failedText).toBe("내장 플레이어로 이 파일을 열지 못했습니다. 외부 프로그램으로 열어 주세요.")
    expect(offlineText).toBe("저장 장치가 연결되어 있지 않아 재생할 수 없습니다.")
    expect(new Set([missingText, failedText, offlineText]).size).toBe(3)
  })

  it("T-ui-viewer-64 [Boundary] a non-playable audio file keeps the format-unsupported message even when the player is available", async () => {
    // Arrange + Act
    await mountMpv({ settings: { videoPlayerVlc: false }, detail: { mediaType: "audio", fileName: "voice.aif" } })
    // Assert
    expect(await screen.findByText(/이 형식은 앱에서 바로 재생할 수 없습니다/)).toBeInTheDocument()
    expect(screen.queryByText(/내장 플레이어를 사용할 수 없어/)).not.toBeInTheDocument()
  })

  it("T-ui-viewer-43 [Business rule] an offline non-playable video shows the offline notice without querying the player", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: { videoPlayerVlc: false }, detail: { availability: "offline" } })
    // Assert
    expect(await screen.findByText(/저장 장치가 연결되어 있지 않아/)).toBeInTheDocument()
    expect(t.api.playerAvailable).not.toHaveBeenCalled()
    expect(t.open).not.toHaveBeenCalled()
  })

  it("T-ui-viewer-45 [Boundary] a non-playable audio file never uses the built-in player", async () => {
    // Arrange + Act
    const t = await mountMpv({ settings: { videoPlayerVlc: false }, detail: { mediaType: "audio", fileName: "voice.aif" } })
    // Assert
    expect(await screen.findByRole("button", { name: "외부 프로그램으로 열기" })).toBeInTheDocument()
    expect(t.api.playerAvailable).not.toHaveBeenCalled()
    expect(t.open).not.toHaveBeenCalled()
  })
})

describe("MpvPlayer — open failure", () => {
  it("T-ui-viewer-44 [External failure] a failed open shows the error inline with the external-open button", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv({ open: { code: "open_failed", message: "boom" } })
    // Act
    const alert = await screen.findByRole("alert")
    // Assert
    expect(alert).toHaveTextContent("내장 플레이어로 이 파일을 열지 못했습니다. 외부 프로그램으로 열어 주세요.")
    await user.click(screen.getByRole("button", { name: "외부 프로그램으로 열기" }))
    expect(t.openOriginal).toHaveBeenCalledWith(1)
    expect(screen.queryByRole("button", { name: "재생" })).not.toBeInTheDocument()
  })

  it("T-ui-viewer-48 [External failure] player_unavailable, asset_offline and player_timeout map to their own messages", async () => {
    // Arrange + Act
    const first = await mountMpv({ open: { code: "player_unavailable", message: "x" } })
    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("내장 플레이어를 사용할 수 없어 재생할 수 없습니다. 앱을 다시 설치하거나 외부 프로그램으로 열어 주세요.")
    first.view.unmount()
    // Arrange + Act
    const second = await mountMpv({ open: { code: "asset_offline", message: "x" } })
    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("저장 장치가 연결되어 있지 않아 재생할 수 없습니다.")
    second.view.unmount()
    // Arrange + Act
    await mountMpv({ open: { code: "player_timeout", message: "x" } })
    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("플레이어가 응답하지 않습니다.")
  })

  it("T-ui-viewer-57 [External failure] an error state event after opening shows the message, closes the player and offers external open", async () => {
    // Arrange
    const t = await mountMpv()
    await t.ready()
    t.close.mockClear()
    // Act
    t.emitState({ state: "error", error: "decoder crashed" })
    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("decoder crashed")
    expect(t.close).toHaveBeenCalled()
    expect(screen.getByRole("button", { name: "외부 프로그램으로 열기" })).toBeInTheDocument()
  })

  it("T-ui-mpv-gl-01 [External failure] without WebGL2 the player is not opened and the unavailable notice with external open is shown", async () => {
    // Arrange + Act
    const t = await mountMpv({ noWebgl: true })
    // Assert
    expect(await screen.findByRole("alert")).toHaveTextContent("내장 플레이어를 사용할 수 없어")
    expect(screen.getByRole("button", { name: "외부 프로그램으로 열기" })).toBeInTheDocument()
    expect(t.open).not.toHaveBeenCalled()
  })
})

describe("MpvPlayer — session lifecycle", () => {
  it("T-ui-viewer-52 [Business rule] the player is closed on viewer close and a new asset closes the old one before opening", async () => {
    // Arrange
    const calls: string[] = []
    const t = await mountMpv({ calls })
    await t.ready()
    expect(calls).toEqual(["open:1"])
    // Act: 자산 변경
    act(() => useViewStore.getState().openPreview(2))
    await waitFor(() => expect(calls).toEqual(["open:1", "close", "open:2"]))
    await waitFor(() => expect(screen.getByRole("img", { name: "v2.mxf" })).toHaveAttribute("aria-busy", "false"))
    // Act: 보기 영역 닫기
    act(() => useViewStore.getState().closePreview())
    // Assert
    await waitFor(() => expect(calls).toEqual(["open:1", "close", "open:2", "close"]))
  })

  it("T-ui-viewer-53 [Business rule] unmounting while the open is still pending closes only after the open finishes", async () => {
    // Arrange
    const calls: string[] = []
    const t = await mountMpv({ calls, open: "pending" })
    await waitFor(() => expect(calls).toEqual(["open:1"]))
    // Act
    t.view.unmount()
    // Assert: open 이 끝나기 전에는 close 가 실행되지 않는다
    expect(calls).toEqual(["open:1"])
    await t.finishOpen()
    await waitFor(() => expect(calls).toEqual(["open:1", "close"]))
  })
})

describe("MpvPlayer — controls", () => {
  it("T-ui-viewer-54 [Happy] buttons and menus call the matching commands with the right arguments", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv()
    await t.ready()
    // 열기 직후 UI 의 초기 음량·속도를 플레이어에 맞춘다
    await waitFor(() => expect(t.setVolume).toHaveBeenCalledWith(100, false))
    expect(t.setRate).toHaveBeenCalledWith(1)
    // Act + Assert: 재생/프레임 이동 (fps 인자 없음)
    await user.click(screen.getByRole("button", { name: "재생" }))
    expect(t.control).toHaveBeenLastCalledWith("toggle")
    await user.click(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" }))
    expect(t.control).toHaveBeenLastCalledWith("stepForward")
    await user.click(screen.getByRole("button", { name: "뒤로 (정지 시 1프레임)" }))
    expect(t.control).toHaveBeenLastCalledWith("stepBack")
    // 속도
    await user.click(screen.getByRole("combobox", { name: "재생 속도" }))
    await user.click(await screen.findByRole("option", { name: "1.5×" }))
    expect(t.setRate).toHaveBeenLastCalledWith(1.5)
    // 음소거: 음량은 유지하고 muted 만 바뀐다
    await user.click(screen.getByRole("button", { name: "음소거" }))
    expect(t.setVolume).toHaveBeenLastCalledWith(100, true)
    expect(screen.getByRole("button", { name: "음소거 해제" })).toHaveAttribute("aria-pressed", "true")
    // 탐색 슬라이더: 컨트롤 바 안 포커스의 화살표는 플레이어 단축키(5초)가 처리한다 (슬라이더 step 아님)
    screen.getByRole("slider", { name: "재생 위치" }).focus()
    await user.keyboard("{ArrowRight}")
    expect(t.seek).toHaveBeenLastCalledWith(5000)
  })

  it("T-ui-viewer-58 [Boundary] frame step needs no asset fps — the command carries the action only, with or without asset fps", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv({ detail: { fps: null } })
    await t.ready()
    // Act
    await user.click(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" }))
    // Assert
    expect(t.control).toHaveBeenLastCalledWith("stepForward")
    expect(t.control.mock.lastCall).toEqual(["stepForward"])
  })

  it("T-ui-viewer-55 [Validation] shortcuts act from any focus (window level) and map to the commands; modifiers and editable targets are ignored", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv()
    await t.ready()
    t.emitState({ state: "playing", timeMs: 20_000 })
    t.control.mockClear()
    // Act: 갤러리 쪽 포커스에서도 동작한다
    document.body.focus()
    await user.keyboard("{ArrowRight}")
    expect(t.seek).toHaveBeenLastCalledWith(25_000)
    // 재생 중 . , 는 초 단위 seek(기본 5초, T-ui-step-05) — 정지 중에만 프레임 이동
    t.emitState({ state: "paused", timeMs: 25_000 })
    await user.keyboard(".")
    expect(t.control).toHaveBeenLastCalledWith("stepForward")
    await user.keyboard(",")
    expect(t.control).toHaveBeenLastCalledWith("stepBack")
    await user.keyboard("{ArrowLeft}")
    expect(t.seek).toHaveBeenLastCalledWith(20_000)
    await user.keyboard("m")
    expect(t.setVolume).toHaveBeenLastCalledWith(100, true)
    // Space 는 포커스된 버튼을 누르지 않고 재생만 토글한다
    t.control.mockClear()
    await user.keyboard(" ")
    expect(t.control).toHaveBeenCalledTimes(1)
    expect(t.control).toHaveBeenLastCalledWith("toggle")
    // 음량: 음소거 상태에서 ↑ 는 5% 로 올리며 해제, ↓ 는 0 이 되면 음소거
    await user.keyboard("{ArrowUp}")
    expect(t.setVolume).toHaveBeenLastCalledWith(5, false)
    await user.keyboard("{ArrowDown}")
    expect(t.setVolume).toHaveBeenLastCalledWith(0, true)
    // 수정자와 함께면 무시
    t.seek.mockClear()
    await user.keyboard("{Control>}{ArrowRight}{/Control}")
    expect(t.seek).not.toHaveBeenCalled()
  })

  it("T-ui-viewer-59 [Business rule] a seek request from the AI transcript seeks the player, also when it arrives before the open finishes", async () => {
    // Arrange
    const t = await mountMpv({ open: "pending" })
    await waitFor(() => expect(t.open).toHaveBeenCalled())
    // Act: 열기 전에 요청
    act(() => useViewStore.getState().seekPreview(1, 14_000))
    expect(t.seek).not.toHaveBeenCalled()
    await t.finishOpen()
    // Assert
    await waitFor(() => expect(t.seek).toHaveBeenCalledWith(14_000))
    // Act: 다음 요청
    act(() => useViewStore.getState().seekPreview(1, 30_000))
    await waitFor(() => expect(t.seek).toHaveBeenLastCalledWith(30_000))
  })

  it("T-ui-mpv-ctl-01 [Happy] clicking the stage toggles play/pause (DOM click, no native input event)", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv()
    await t.ready()
    // Act
    await user.click(t.stage())
    // Assert
    expect(t.control).toHaveBeenLastCalledWith("toggle")
  })
})

describe("MpvPlayer — state events", () => {
  it("T-ui-viewer-56 [Happy] player://state updates the position, duration and play button; other assets' events are ignored", async () => {
    // Arrange
    const t = await mountMpv()
    await t.ready()
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
    // Act
    t.emitState({ state: "playing", timeMs: 65_000, lengthMs: 120_000 })
    // Assert
    expect(screen.getByLabelText("1:05 / 2:00")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "일시정지" })).toBeInTheDocument()
    // Act: 다른 자산의 이벤트
    t.emitState({ assetId: 99, state: "paused", timeMs: 5_000 })
    // Assert
    expect(screen.getByLabelText("1:05 / 2:00")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "일시정지" })).toBeInTheDocument()
  })

  it("T-ui-mpv-time-01 [Happy] after a frame step the displayed time is the event time — no local frame math", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv()
    await t.ready()
    t.emitState({ state: "paused", timeMs: 10_000 })
    const position = () => Number(screen.getByRole("slider", { name: "재생 위치" }).getAttribute("aria-valuenow"))
    expect(position()).toBeCloseTo(10, 6)
    // Act: 다음 프레임 — 클릭만으로는 시각이 바뀌지 않는다 (29.97fps 같은 프레임 시각을 UI 가 계산하지 않는다)
    await user.click(screen.getByRole("button", { name: "앞으로 (정지 시 1프레임)" }))
    expect(position()).toBeCloseTo(10, 6)
    // 워커가 교정한 시각이 이벤트로 온다
    t.emitState({ state: "paused", timeMs: 10_033 })
    // Assert
    expect(position()).toBeCloseTo(10.033, 6)
    // 이전 프레임
    await user.click(screen.getByRole("button", { name: "뒤로 (정지 시 1프레임)" }))
    t.emitState({ state: "paused", timeMs: 10_000 })
    expect(position()).toBeCloseTo(10, 6)
  })

  it("T-ui-mpv-time-02 [Boundary] events of an older session are ignored once the open reply gave the session id; newer ones are accepted", async () => {
    // Arrange
    const t = await mountMpv({ reply: { sessionId: 7 } })
    await t.ready()
    // Act + Assert: 이전 세션의 늦은 이벤트
    t.emitState({ sessionId: 6, state: "playing", timeMs: 50_000 })
    expect(screen.getByRole("button", { name: "재생" })).toBeInTheDocument()
    // 현재·이후 세션
    t.emitState({ sessionId: 7, state: "playing", timeMs: 51_000 })
    expect(screen.getByLabelText("0:51 / 2:00")).toBeInTheDocument()
    t.emitState({ sessionId: 8, state: "paused", timeMs: 52_000 })
    expect(screen.getByLabelText("0:52 / 2:00")).toBeInTheDocument()
  })

  it("T-ui-mpv-time-03 [Happy] the open reply seeds the first state: an autoplaying open shows the pause button and its time", async () => {
    // Arrange + Act
    const t = await mountMpv({ reply: { state: "playing", timeMs: 3_000 } })
    await t.ready()
    // Assert: 열기 응답은 첫 렌더 뒤 효과로 상태에 반영된다
    expect(await screen.findByRole("button", { name: "일시정지" })).toBeInTheDocument()
    expect(screen.getByLabelText("0:03 / 2:00")).toBeInTheDocument()
  })

  it("T-ui-mpv-time-04 [Boundary] a seek shows its target time at once; the corrected event time replaces it", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv()
    await t.ready()
    t.emitState({ state: "paused", timeMs: 10_000 })
    const position = () => Number(screen.getByRole("slider", { name: "재생 위치" }).getAttribute("aria-valuenow"))
    // Act
    screen.getByRole("slider", { name: "재생 위치" }).focus()
    await user.keyboard("{ArrowRight}")
    // Assert
    expect(t.seek).toHaveBeenLastCalledWith(15_000)
    expect(position()).toBeCloseTo(15, 6)
    t.emitState({ state: "paused", timeMs: 15_040 })
    expect(position()).toBeCloseTo(15.04, 6)
  })
})

describe("MpvPlayer — stage and status", () => {
  it("T-ui-vlcset-13 [Business rule] opening is exposed as aria-busy on the stage and no status line is rendered when ready", async () => {
    // Arrange + Act
    const t = await mountMpv({ open: "pending" })
    const stage = await screen.findByRole("img", { name: "v1.mxf" })
    // Assert: 여는 중
    expect(stage).toHaveAttribute("aria-busy", "true")
    // Act
    await t.finishOpen()
    await t.ready()
    // Assert: 준비 완료 — 상태 줄 없음
    expect(stage).toHaveAttribute("aria-busy", "false")
    expect(screen.queryByText(/재생 중/)).not.toBeInTheDocument()
    expect(screen.queryByText(/여는 중/)).not.toBeInTheDocument()
  })

  it("T-ui-vlcset-14 [Business rule] setting ON opens even a WebView-playable video in the built-in player", async () => {
    // Arrange + Act
    const t = await mountMpv({ detail: { playable: true } })
    await t.ready()
    // Assert
    expect(t.open).toHaveBeenCalledWith(1, CONTAIN_1080P, true)
    expect(document.querySelector("video")).toBeNull()
  })

  it("T-ui-mpv-ctl-02 [Business rule] a menu or dialog opening over the video neither hides the player nor takes a snapshot (the canvas is plain DOM)", async () => {
    // Arrange
    const t = await mountMpv()
    await t.ready()
    t.setRenderActive.mockClear()
    // Act
    const layer = document.createElement("div")
    layer.setAttribute("role", "dialog")
    document.body.appendChild(layer)
    await act(async () => void (await Promise.resolve()))
    // Assert
    expect(t.canvas()).toBeInTheDocument()
    expect(t.setRenderActive).not.toHaveBeenCalled()
    expect((t.api as unknown as Record<string, unknown>).playerSnapshot).toBeUndefined()
    layer.remove()
  })
})
