// Categories: Happy (T-ui-playerkeys-10..11) / Business rule (12..14) / Validation (15..17: editable, layer, modifiers)
//   control-bar focus (18..23: Business rule / Validation / Happy IME) / wheel volume (T-ui-volwheel-01..03: Happy / Boundary / Business rule)
import { act, fireEvent, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { AssetDetail } from "@/lib/ipc-contract"
import { createMockApi } from "@/lib/api/mock"
import { useSelectionStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 갤러리 목록(listbox)에 포커스가 있는 채로, 보기 영역에 patch 형태의 자산이 열린 상태 */
async function setup(patch: Partial<AssetDetail>, open = true) {
  stubLayout()
  const media = stubMedia()
  const api = createMockApi({ count: 50 })
  await useStubbedDetail(api, patch)
  const user = userEvent.setup()
  renderApp(api)
  const list = await screen.findByRole("listbox", { name: "미디어 목록" })
  const [first] = await within(list).findAllByRole("option")
  if (open) {
    act(() => useViewStore.getState().openPreview(1))
    await user.click(first) // 선택, 포커스는 목록에 남는다
    expect(useViewStore.getState().previewId).not.toBeNull()
  } else act(() => list.focus())
  return { media, user, list, selected: () => [...useSelectionStore.getState().ids] }
}

describe("player keys take priority over the gallery while a player is open", () => {
  it("T-ui-playerkeys-10 [Happy] video open, focus on the gallery list: arrows seek, up/down change volume 5%, Space toggles play, selection untouched", async () => {
    // Arrange
    const { media, user, list, selected } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 20
    video.volume = 0.5
    const before = selected()
    act(() => list.focus())
    // Act
    await user.keyboard("{ArrowRight}")
    const afterRight = video.currentTime
    await user.keyboard("{ArrowLeft}")
    await user.keyboard("{ArrowUp}")
    const afterUp = video.volume
    await user.keyboard("{ArrowDown}{ArrowDown}")
    await user.keyboard(" ")
    // Assert
    expect(afterRight).toBe(25)
    expect(video.currentTime).toBe(20)
    expect(afterUp).toBeCloseTo(0.55, 6)
    expect(video.volume).toBeCloseTo(0.45, 6)
    expect(media.play).toHaveBeenCalledTimes(1)
    expect(selected()).toEqual(before)
  })

  it("T-ui-playerkeys-11 [Business rule] volume: lowering to 0 mutes, raising from muted unmutes at 5%; clamps at 1", async () => {
    // Arrange
    const { user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.volume = 0.05
    // Act + Assert
    await user.keyboard("{ArrowDown}")
    expect(video.volume).toBe(0)
    expect(video.muted).toBe(true)
    await user.keyboard("{ArrowUp}")
    expect(video.volume).toBeCloseTo(0.05, 6)
    expect(video.muted).toBe(false)
    video.volume = 1
    await user.keyboard("{ArrowUp}")
    expect(video.volume).toBe(1)
  })

  it("T-ui-playerkeys-12 [Business rule] audio open: same keys, but comma/period (frame step) are ignored", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "audio", fileName: "song.mp3" })
    const audio = await screen.findByLabelText<HTMLAudioElement>("song.mp3", { selector: "audio" })
    audio.currentTime = 20
    // Act
    await user.keyboard(".,")
    const afterStep = audio.currentTime
    await user.keyboard("{ArrowRight}")
    await user.keyboard(" ")
    await user.keyboard("m")
    // Assert
    expect(afterStep).toBe(20)
    expect(media.pause).not.toHaveBeenCalled()
    expect(audio.currentTime).toBe(25)
    expect(media.play).toHaveBeenCalledTimes(1)
    expect(audio.muted).toBe(true)
  })

  it("T-ui-playerkeys-13 [Business rule] image viewer open: arrows still move the gallery cursor (no player registered)", async () => {
    // Arrange
    const { user, selected } = await setup({ mediaType: "image", fileName: "photo.jpg" })
    const before = selected()
    // Act
    await user.keyboard("{ArrowRight}")
    // Assert
    expect(selected()).not.toEqual(before)
  })

  it("T-ui-playerkeys-14 [Business rule] no viewer open: gallery arrows and Space work as before", async () => {
    // Arrange
    const { user, media, selected } = await setup({ mediaType: "video", fileName: "clip.mp4" }, false)
    // Act
    await user.keyboard("{ArrowDown}")
    // Assert
    expect(selected()).toHaveLength(1)
    expect(media.play).not.toHaveBeenCalled()
  })

  it("T-ui-playerkeys-15 [Validation] focus in an input: player keys do nothing", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 20
    act(() => screen.getByRole("combobox", { name: "검색" }).focus())
    // Act
    await user.keyboard(" {ArrowRight}")
    // Assert
    expect(video.currentTime).toBe(20)
    expect(media.play).not.toHaveBeenCalled()
  })

  it("T-ui-playerkeys-16 [Validation] an open dialog layer keeps its own keys", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 20
    const layer = document.createElement("div")
    layer.setAttribute("role", "dialog")
    document.body.appendChild(layer)
    // Act
    await user.keyboard(" {ArrowRight}")
    layer.remove()
    // Assert
    expect(video.currentTime).toBe(20)
    expect(media.play).not.toHaveBeenCalled()
  })

  it("T-ui-playerkeys-17 [Validation] modified keys are ignored; Space on a focused button does not also click it", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 20
    // Act: 수정자
    await user.keyboard("{Control>}{ArrowRight}{/Control}{Shift>}{ArrowRight}{/Shift}{Alt>}m{/Alt}")
    expect(video.currentTime).toBe(20)
    expect(video.muted).toBe(false)
    // Act: 버튼 포커스 + Space → 재생 토글 한 번
    const playButton = screen.getAllByRole("button").find((b) => b.getAttribute("aria-label") === "재생") as HTMLElement
    act(() => playButton.focus())
    await user.keyboard(" ")
    // Assert
    expect(media.play).toHaveBeenCalledTimes(1)
  })
})

describe("focus inside the control bar (regression: Space died after dragging the seek bar)", () => {
  it("T-ui-playerkeys-18 [Business rule] seek slider focused: Space toggles play, arrows seek via player (not slider step)", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 20
    act(() => screen.getByRole("slider", { name: "재생 위치" }).focus())
    // Act
    await user.keyboard(" ")
    await user.keyboard("{ArrowRight}")
    // Assert
    expect(media.play).toHaveBeenCalledTimes(1)
    expect(video.currentTime).toBe(25) // 슬라이더 step(0.1)이 아니라 플레이어 SEEK_STEP
  })

  it("T-ui-playerkeys-19 [Business rule] volume slider focused: ArrowUp changes volume by the player step; M mutes", async () => {
    // Arrange
    const { user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.volume = 0.5
    act(() => screen.getByRole("slider", { name: "음량" }).focus())
    // Act
    await user.keyboard("{ArrowUp}")
    const afterUp = video.volume
    await user.keyboard("m")
    // Assert
    expect(afterUp).toBeCloseTo(0.55, 6)
    expect(video.muted).toBe(true)
  })

  it("T-ui-playerkeys-20 [Business rule] rate select trigger focused: Space toggles play and does not open the select", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    act(() => screen.getByRole("combobox", { name: "재생 속도" }).focus())
    // Act
    await user.keyboard(" ")
    // Assert
    expect(media.play).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole("listbox", { name: /./, hidden: false })?.getAttribute("data-state")).not.toBe("open")
    expect(screen.getByRole("combobox", { name: "재생 속도" })).toHaveAttribute("aria-expanded", "false")
  })

  it("T-ui-playerkeys-21 [Validation] a slider outside the control bar keeps its own keys", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.currentTime = 20
    const outside = document.createElement("div")
    outside.setAttribute("role", "slider")
    outside.tabIndex = 0
    document.body.appendChild(outside)
    act(() => outside.focus())
    // Act
    await user.keyboard("{ArrowRight}")
    await user.keyboard(" ")
    outside.remove()
    // Assert
    expect(video.currentTime).toBe(20)
    expect(media.play).not.toHaveBeenCalled()
  })

  it("T-ui-playerkeys-22 [Validation] an open select listbox is ignored even with the seek slider focused", async () => {
    // Arrange
    const { media, user } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    act(() => screen.getByRole("slider", { name: "재생 위치" }).focus())
    const layer = document.createElement("div")
    layer.setAttribute("role", "listbox")
    layer.setAttribute("data-state", "open")
    document.body.appendChild(layer)
    // Act
    await user.keyboard(" ")
    layer.remove()
    // Assert
    expect(media.play).not.toHaveBeenCalled()
  })

  it("T-ui-playerkeys-23 [Happy] Korean IME (key 'ㅡ', code KeyM) mutes; Shift+M does not", async () => {
    // Arrange
    await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Act
    fireEvent.keyDown(document.body, { key: "ㅡ", code: "KeyM", shiftKey: true })
    const afterShift = video.muted
    fireEvent.keyDown(document.body, { key: "ㅡ", code: "KeyM" })
    // Assert
    expect(afterShift).toBe(false)
    expect(video.muted).toBe(true)
  })
})

describe("mouse wheel over the stage changes volume", () => {
  afterEach(() => vi.useRealTimers())

  it("T-ui-volwheel-01 [Happy] wheel up/down moves volume by 5% and shows a status indicator that auto-hides", async () => {
    // Arrange
    await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.volume = 0.5
    const stage = video.parentElement as HTMLElement
    vi.useFakeTimers()
    // Act
    fireEvent.wheel(stage, { deltaY: -100 })
    const afterUp = video.volume
    const shown = screen.getByText(/음량 [0-9]+%/).textContent
    fireEvent.wheel(stage, { deltaY: 100 })
    fireEvent.wheel(stage, { deltaY: 100 })
    const afterDown = video.volume
    act(() => void vi.advanceTimersByTime(1000))
    // Assert
    expect(afterUp).toBeCloseTo(0.55, 6)
    expect(shown).toContain("55%")
    expect(afterDown).toBeCloseTo(0.45, 6)
    expect(screen.queryByText(/음량 [0-9]+%/)).toBeNull()
  })

  it("T-ui-volwheel-02 [Boundary] clamps at 1 and at 0 (reaching 0 mutes)", async () => {
    // Arrange
    await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    const stage = video.parentElement as HTMLElement
    video.volume = 1
    // Act + Assert
    fireEvent.wheel(stage, { deltaY: -100 })
    expect(video.volume).toBe(1)
    video.volume = 0.05
    fireEvent.wheel(stage, { deltaY: 100 })
    fireEvent.wheel(stage, { deltaY: 100 })
    expect(video.volume).toBe(0)
    expect(video.muted).toBe(true)
  })

  it("T-ui-volwheel-03 [Business rule] wheel is cancelable (non-passive, page does not scroll) and does not toggle play", async () => {
    // Arrange
    const { media } = await setup({ mediaType: "video", fileName: "clip.mp4" })
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Act
    const notPrevented = fireEvent.wheel(video.parentElement as HTMLElement, { deltaY: -100 })
    // Assert
    expect(notPrevented).toBe(false)
    expect(media.play).not.toHaveBeenCalled()
  })
})
