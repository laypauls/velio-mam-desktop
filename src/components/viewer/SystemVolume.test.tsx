// Categories: Happy (T-ui-sysvol-01..04: built-in player input / Shift+arrows / web stage wheel -> system volume) / Business rule (T-ui-sysvol-05 Shift+N keeps working, app volume untouched) / External (T-ui-sysvol-06 audio_device_unavailable: indicator + toast, no app-volume fallback)
import { act, fireEvent, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { usePlaybackStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { mountMpv, mpvQueueItem } from "@/test/mpv"

vi.mock("sonner", async (importOriginal) => ({
  ...(await importOriginal<typeof import("sonner")>()),
  toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() },
}))

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("built-in player player: Shift = system volume", () => {
  it("T-ui-sysvol-01 [Happy] a DOM wheel with Shift steps the system volume (not the app volume) and shows 시스템 음량 n%; shift=false stays app volume", async () => {
    // Arrange
    const t = await mountMpv()
    const step = vi.spyOn(t.api, "systemVolumeStep").mockResolvedValue(0.6)
    await t.ready()
    t.setVolume.mockClear()
    // Act 1: Shift + 위로 2노치
    fireEvent.wheel(t.stage(), { deltaY: -200, shiftKey: true })
    // Assert 1
    expect(step).toHaveBeenCalledTimes(1)
    expect(step.mock.calls[0][0]).toBeCloseTo(0.1, 9)
    expect(await screen.findByText("시스템 음량 60%")).toBeInTheDocument()
    expect(t.setVolume).not.toHaveBeenCalled()
    // Act 2: Shift 없이 아래로 1노치
    fireEvent.wheel(t.stage(), { deltaY: 100, shiftKey: false })
    // Assert 2
    expect(t.setVolume).toHaveBeenCalledTimes(1)
    expect(step).toHaveBeenCalledTimes(1)
  })

  it("T-ui-sysvol-02 [Happy] Shift+ArrowUp / ArrowDown step the system volume ±5% and leave the app volume alone", async () => {
    // Arrange
    const t = await mountMpv()
    const step = vi.spyOn(t.api, "systemVolumeStep").mockResolvedValue(0.55)
    await t.ready()
    const user = userEvent.setup()
    t.setVolume.mockClear()
    // Act
    await user.keyboard("{Shift>}{ArrowUp}{ArrowDown}{/Shift}")
    // Assert
    expect(step.mock.calls.map((c) => Math.round(c[0] / 0.05))).toEqual([1, -1])
    expect(t.setVolume).not.toHaveBeenCalled()
    expect(await screen.findByText("시스템 음량 55%")).toBeInTheDocument()
  })

  it("T-ui-sysvol-05 [Business rule] Shift+N still moves to the next queue item and does not step the system volume", async () => {
    // Arrange
    const t = await mountMpv({ queue: [mpvQueueItem(1), mpvQueueItem(2)] })
    const step = vi.spyOn(t.api, "systemVolumeStep").mockResolvedValue(0.5)
    await t.ready()
    const user = userEvent.setup()
    // Act
    await user.keyboard("{Shift>}n{/Shift}")
    // Assert
    expect(usePlaybackStore.getState().index).toBe(1)
    expect(step).not.toHaveBeenCalled()
  })

  it("T-ui-sysvol-06 [External] audio_device_unavailable shows the error in the indicator and as one toast; the app volume is NOT used as a fallback", async () => {
    // Arrange
    const t = await mountMpv()
    const step = vi.spyOn(t.api, "systemVolumeStep").mockRejectedValue({ code: "audio_device_unavailable", message: "no output device" })
    await t.ready()
    t.setVolume.mockClear()
    const message = "시스템 음량을 조정할 수 없습니다. 출력 장치를 확인하세요."
    // Act
    fireEvent.wheel(t.stage(), { deltaY: -100, shiftKey: true })
    // Assert
    expect(await screen.findByText(message)).toBeInTheDocument()
    expect(step).toHaveBeenCalledTimes(1)
    expect(toast.error).toHaveBeenCalledWith(message, { id: "system-volume" })
    expect(t.setVolume).not.toHaveBeenCalled()
  })
})

describe("HTML player: Shift = system volume", () => {
  async function setup() {
    stubLayout()
    stubMedia()
    const api = createMockApi({ count: 50 })
    await useStubbedDetail(api, { mediaType: "video", fileName: "clip.mp4" })
    const step = vi.spyOn(api, "systemVolumeStep").mockResolvedValue(0.45)
    renderApp(api)
    await screen.findByRole("listbox", { name: "미디어 목록" })
    act(() => useViewStore.getState().openPreview(1))
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    video.volume = 0.5
    return { step, video, stage: video.parentElement as HTMLElement }
  }

  it("T-ui-sysvol-03 [Happy] Shift+wheel on the stage (deltaY and Chromium's deltaX form) steps the system volume; plain wheel changes the element (app) volume", async () => {
    // Arrange
    const { step, video, stage } = await setup()
    // Act: deltaY 형태
    const prevented = !fireEvent.wheel(stage, { deltaY: -100, shiftKey: true })
    // Assert
    expect(prevented).toBe(true)
    expect(step).toHaveBeenCalledTimes(1)
    expect(step.mock.calls[0][0]).toBeCloseTo(0.05, 9)
    expect(video.volume).toBeCloseTo(0.5, 6)
    expect(await screen.findByText("시스템 음량 45%")).toBeInTheDocument()
    // Act: deltaX 형태 (아래)
    fireEvent.wheel(stage, { deltaX: 100, deltaY: 0, shiftKey: true })
    expect(step).toHaveBeenCalledTimes(2)
    expect(step.mock.calls[1][0]).toBeCloseTo(-0.05, 9)
    expect(video.volume).toBeCloseTo(0.5, 6)
    // Act: Shift 없음
    fireEvent.wheel(stage, { deltaY: -100 })
    // Assert
    expect(video.volume).toBeCloseTo(0.55, 6)
    expect(step).toHaveBeenCalledTimes(2)
  })

  it("T-ui-sysvol-04 [Happy] Shift+ArrowUp on the HTML player steps the system volume, not video.volume", async () => {
    // Arrange
    const { step, video } = await setup()
    const user = userEvent.setup()
    // Act
    await user.keyboard("{Shift>}{ArrowUp}{/Shift}")
    // Assert
    expect(step).toHaveBeenCalledTimes(1)
    expect(video.volume).toBeCloseTo(0.5, 6)
  })
})
