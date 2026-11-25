// Categories: Happy (T-ui-fullscreen-01 button, 02 F key, 09 image) / Business rule (T-ui-fullscreen-03 Esc exits only, 04 Q exits + closes, 05 any close path exits, 06 hide shortcut, 10 hidden window) / Time (T-ui-fullscreen-07 idle hide) / External failure (T-ui-fullscreen-08 enter/exit reject) / Concurrency (T-ui-fullscreen-11 no remount)
import { act, fireEvent, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { AssetDetail } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { FULLSCREEN_IDLE_MS } from "./useFullscreenIdle"

const win = vi.hoisted(() => ({
  setFullscreen: vi.fn(),
  isMaximized: vi.fn(),
  maximize: vi.fn(),
  setTheme: vi.fn(),
}))
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => win }))

beforeEach(() => {
  win.setFullscreen.mockReset().mockResolvedValue(undefined)
  win.isMaximized.mockReset().mockResolvedValue(false)
  win.maximize.mockReset().mockResolvedValue(undefined)
  win.setTheme.mockReset().mockResolvedValue(undefined)
})

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(opts: { hideShortcut?: string | null; detail?: Partial<AssetDetail> } = {}) {
  stubLayout()
  stubMedia()
  const api = createMockApi({ count: 20 })
  await useStubbedDetail(api, opts.detail)
  if (opts.hideShortcut !== undefined) await api.updateSettings({ hideShortcut: opts.hideShortcut })
  const hide = vi.spyOn(api, "hideMainWindow").mockResolvedValue(undefined)
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  act(() => useViewStore.getState().openPreview(1))
  const video = opts.detail?.mediaType === "image" ? null : await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
  return { api, hide, user, video }
}

const pane = () => screen.getByRole("complementary", { name: "콘텐츠 보기" })
const calls = () => win.setFullscreen.mock.calls.map((c) => c[0])

describe("fullscreen entry", () => {
  it("T-ui-fullscreen-01 [Happy] the control-row button enters and leaves fullscreen; the pane covers the window and the label/pressed state follow", async () => {
    // Arrange
    const { user } = await setup()
    // Act
    await user.click(screen.getByRole("button", { name: "전체화면" }))
    // Assert
    await waitFor(() => expect(calls()).toEqual([true]))
    expect(useViewStore.getState().fullscreen).toBe(true)
    expect(pane().className).toContain("fixed")
    expect(pane().className).toContain("inset-0")
    expect(screen.queryByRole("button", { name: "보기 영역 닫기" })).not.toBeInTheDocument() // 머리글(닫기 버튼)은 숨김
    const exit = screen.getByRole("button", { name: "전체화면 해제" })
    expect(exit).toHaveAttribute("aria-pressed", "true")
    // Act
    await user.click(exit)
    // Assert
    await waitFor(() => expect(calls()).toEqual([true, false]))
    expect(pane().className).not.toContain("fixed")
    expect(screen.getByRole("button", { name: "보기 영역 닫기" })).toBeInTheDocument()
  })

  it("T-ui-fullscreen-02 [Happy] F toggles (physical key code, so a Korean IME key still works); typing F in an input does not", async () => {
    // Arrange
    const { user } = await setup()
    // Act: 한글 IME 상태의 F 키 — key 는 "ㄹ", code 는 KeyF
    fireEvent.keyDown(window, { key: "ㄹ", code: "KeyF" })
    // Assert
    await waitFor(() => expect(calls()).toEqual([true]))
    // Act: 영문 f 로 해제
    await user.keyboard("f")
    await waitFor(() => expect(calls()).toEqual([true, false]))
    // Act: 검색 입력 중의 f 는 글자 입력이다
    await user.click(screen.getByRole("combobox", { name: "검색" }))
    await user.keyboard("f")
    // Assert
    expect(calls()).toEqual([true, false])
    expect(useViewStore.getState().fullscreen).toBe(false)
  })

  it("T-ui-fullscreen-09 [Happy] images get the same button and F key", async () => {
    // Arrange
    const { user } = await setup({ detail: { mediaType: "image", fileName: "photo.jpg" } })
    await screen.findByTestId("image-stage")
    // Act
    await user.click(screen.getByRole("button", { name: "전체화면" }))
    // Assert
    await waitFor(() => expect(calls()).toEqual([true]))
    expect(pane().className).toContain("fixed")
    // Act
    await user.keyboard("f")
    await waitFor(() => expect(calls()).toEqual([true, false]))
  })

  it("T-ui-fullscreen-11 [Concurrency] entering and leaving fullscreen keeps the same media element (no remount, so playback is not restarted)", async () => {
    // Arrange
    const { user, video } = await setup()
    // Act
    await user.click(screen.getByRole("button", { name: "전체화면" }))
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    // Assert
    expect(screen.getByLabelText("clip.mp4", { selector: "video" })).toBe(video)
    // Act
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(false))
    expect(screen.getByLabelText("clip.mp4", { selector: "video" })).toBe(video)
  })
})

describe("fullscreen keys", () => {
  it("T-ui-fullscreen-03 [Business rule] Esc leaves fullscreen only - the viewer stays open and the app is not hidden even though Esc is the default hide key", async () => {
    // Arrange: 기본 숨기기 키 = Esc
    const { user, hide } = await setup()
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    // Act
    await user.keyboard("{Escape}")
    // Assert
    await waitFor(() => expect(calls()).toEqual([true, false]))
    expect(useViewStore.getState().fullscreen).toBe(false)
    expect(useViewStore.getState().previewId).toBe(1)
    expect(hide).not.toHaveBeenCalled()
  })

  it("T-ui-fullscreen-03b [Business rule, gate OFF] with the hide shortcut disabled the viewer Esc handler still leaves fullscreen only; the next Esc closes the viewer", async () => {
    // Arrange
    const { user } = await setup({ hideShortcut: null })
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    // Act + Assert: 첫 Esc = 해제만
    await user.keyboard("{Escape}")
    await waitFor(() => expect(calls()).toEqual([true, false]))
    expect(useViewStore.getState().previewId).toBe(1)
    // Act + Assert: 두 번째 Esc = 기존 동작(닫기)
    await user.keyboard("{Escape}")
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-fullscreen-04 [Business rule] Q (close-viewer shortcut) leaves fullscreen and closes the viewer", async () => {
    // Arrange
    const { user } = await setup()
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    // Act
    await user.keyboard("q")
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
    expect(useViewStore.getState().fullscreen).toBe(false)
    await waitFor(() => expect(calls()).toEqual([true, false]))
  })

  it("T-ui-fullscreen-05 [Business rule] every close path leaves fullscreen (store closePreview, e.g. playback end / asset removed)", async () => {
    // Arrange
    await setup()
    act(() => useViewStore.getState().setFullscreen(true))
    await waitFor(() => expect(calls()).toEqual([true]))
    // Act
    act(() => useViewStore.getState().closePreview())
    // Assert
    expect(useViewStore.getState().fullscreen).toBe(false)
    await waitFor(() => expect(calls()).toEqual([true, false]))
  })

  it("T-ui-fullscreen-05b [Business rule] a viewer that disappears without closePreview (unmount) still releases the window", async () => {
    // Arrange
    const { user } = await setup()
    await user.keyboard("f")
    await waitFor(() => expect(calls()).toEqual([true]))
    // Act: 설정 화면 진입 등으로 보기 영역이 사라지는 경로
    act(() => useViewStore.setState({ previewId: null }))
    // Assert
    await waitFor(() => expect(calls()).toEqual([true, false]))
    expect(useViewStore.getState().fullscreen).toBe(false)
  })

  it("T-ui-fullscreen-06 [Business rule, gate ON] a non-Esc hide shortcut leaves fullscreen first and then hides", async () => {
    // Arrange
    const { user, hide } = await setup({ hideShortcut: "Ctrl+Shift+H" })
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    // Act
    await user.keyboard("{Control>}{Shift>}h{/Shift}{/Control}")
    // Assert
    expect(hide).toHaveBeenCalledTimes(1)
    expect(useViewStore.getState().fullscreen).toBe(false)
    await waitFor(() => expect(calls()).toEqual([true, false]))
  })

  it("T-ui-fullscreen-10 [Business rule] a window hidden by the tray or minimized (document hidden) leaves fullscreen", async () => {
    // Arrange
    const { user } = await setup()
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    const visibility = vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden")
    // Act
    act(() => void document.dispatchEvent(new Event("visibilitychange")))
    // Assert
    expect(useViewStore.getState().fullscreen).toBe(false)
    await waitFor(() => expect(calls()).toEqual([true, false]))
    visibility.mockRestore()
  })

  it("T-ui-fullscreen-12 [Business rule] playback keys keep working in fullscreen (Space toggles play)", async () => {
    // Arrange
    const { user, video } = await setup()
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    const play = vi.spyOn(video as HTMLVideoElement, "play")
    // Act
    await user.keyboard(" ")
    // Assert
    expect(play).toHaveBeenCalled()
  })
})

describe("fullscreen idle", () => {
  it("T-ui-fullscreen-07 [Time] controls and cursor hide after 2.5s without mouse movement and return on movement; never outside fullscreen", async () => {
    // Arrange
    await setup()
    vi.useFakeTimers()
    act(() => useViewStore.getState().setFullscreen(true))
    expect(pane().dataset.idle).toBeUndefined()
    // Act: 2.4초 — 아직
    act(() => void vi.advanceTimersByTime(FULLSCREEN_IDLE_MS - 100))
    expect(pane().dataset.idle).toBeUndefined()
    // Act: 2.5초
    act(() => void vi.advanceTimersByTime(100))
    // Assert
    expect(pane().dataset.idle).toBe("true")
    expect(pane().className).toContain("cursor-none")
    // Act: 마우스 이동 → 다시 보임, 또 2.5초 뒤 숨김
    act(() => void fireEvent.mouseMove(window))
    expect(pane().dataset.idle).toBeUndefined()
    act(() => void vi.advanceTimersByTime(FULLSCREEN_IDLE_MS))
    expect(pane().dataset.idle).toBe("true")
    // Act: 전체화면 해제 → 즉시 해제
    act(() => useViewStore.getState().setFullscreen(false))
    expect(pane().dataset.idle).toBeUndefined()
    act(() => void vi.advanceTimersByTime(FULLSCREEN_IDLE_MS * 2))
    expect(pane().dataset.idle).toBeUndefined()
  })

  it("T-ui-fullscreen-07b [Boundary] an open menu or dialog keeps the controls visible", async () => {
    // Arrange
    await setup()
    vi.useFakeTimers()
    act(() => useViewStore.getState().setFullscreen(true))
    const menu = document.createElement("div")
    menu.setAttribute("role", "menu")
    document.body.appendChild(menu)
    // Act
    act(() => void vi.advanceTimersByTime(FULLSCREEN_IDLE_MS * 2))
    // Assert
    expect(pane().dataset.idle).toBeUndefined()
    // Act: 메뉴가 닫힌 뒤
    menu.remove()
    act(() => void vi.advanceTimersByTime(FULLSCREEN_IDLE_MS))
    expect(pane().dataset.idle).toBe("true")
  })
})

describe("fullscreen failure", () => {
  it("T-ui-fullscreen-08 [External failure] a rejected enter leaves the UI in normal mode and tells the user", async () => {
    // Arrange
    const { user } = await setup()
    const error = vi.spyOn(toast, "error").mockReturnValue("t")
    win.setFullscreen.mockRejectedValueOnce(new Error("denied"))
    // Act
    await user.click(screen.getByRole("button", { name: "전체화면" }))
    // Assert: 상태가 창의 실제 상태(일반)로 되돌아온다
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(false))
    expect(error).toHaveBeenCalledWith("전체화면을 전환하지 못했습니다.")
    expect(pane().className).not.toContain("fixed")
    expect(screen.getByRole("button", { name: "전체화면" })).toHaveAttribute("aria-pressed", "false")
  })

  it("T-ui-fullscreen-08b [External failure] a rejected exit keeps the UI in fullscreen (matches the window) so the user can retry", async () => {
    // Arrange
    const { user } = await setup()
    const error = vi.spyOn(toast, "error").mockReturnValue("t")
    await user.keyboard("f")
    await waitFor(() => expect(calls()).toEqual([true]))
    win.setFullscreen.mockRejectedValueOnce(new Error("denied"))
    // Act
    await user.keyboard("f")
    // Assert
    await waitFor(() => expect(error).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(true))
    expect(useViewStore.getState().previewId).toBe(1)
    // Act: 재시도
    await user.keyboard("f")
    await waitFor(() => expect(useViewStore.getState().fullscreen).toBe(false))
  })
})

describe("fullscreen store", () => {
  it("T-ui-fullscreen-13 [Validation] fullscreen cannot be set while no viewer is open, and closePreview always resets it", () => {
    // Act + Assert
    useViewStore.setState({ previewId: null, fullscreen: false })
    useViewStore.getState().setFullscreen(true)
    expect(useViewStore.getState().fullscreen).toBe(false)
    useViewStore.setState({ previewId: 3 })
    useViewStore.getState().setFullscreen(true)
    expect(useViewStore.getState().fullscreen).toBe(true)
    useViewStore.getState().closePreview()
    expect(useViewStore.getState().fullscreen).toBe(false)
  })
})
