// Categories: Happy (wheel over slider / mute button, indicator) / Business rule (Shift = system, shared accumulator, HTML + built-in player) / Boundary (no scroll, outside area ignored, no focus move)
import { act, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { mountMpv } from "@/test/mpv"
import { MediaControls } from "./MediaControls"
import type { MediaState } from "./useMediaState"
import { VolumeIndicator, useVolumeWheel } from "./useVolumeWheel"
import { useVolumeNotches } from "./wheelNotches"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const STATE: MediaState = { currentTime: 0, duration: 100, paused: true, rate: 1, volume: 0.5, muted: false }

describe("VolumeIndicator — icon + percent on one line", () => {
  it("T-ui-volind-01 [Happy] shows only '55%' visibly; the full text stays as screen-reader text, app vs system differ by icon", () => {
    // Arrange + Act
    const { container, rerender } = render(<VolumeIndicator level={0.55} />)
    const appIcon = container.querySelector("svg")?.getAttribute("class")
    // Assert: 앱 음량
    const status = screen.getByRole("status")
    expect(status).toHaveTextContent("음량 55%")
    expect(screen.getByText("55%")).toHaveAttribute("aria-hidden", "true")
    expect(screen.getByText("음량 55%")).toHaveClass("sr-only")
    // Act: 시스템 음량
    rerender(<VolumeIndicator level={0.55} system />)
    // Assert: 문구는 스크린리더용, 아이콘은 앱 음량과 다르다
    expect(screen.getByText("시스템 음량 55%")).toHaveClass("sr-only")
    expect(container.querySelector("svg")?.getAttribute("class")).not.toBe(appIcon)
  })

  it("T-ui-volind-02 [Boundary] level 0 uses the muted icon for app volume; hidden when there is neither level nor error", () => {
    // Arrange
    const { container, rerender } = render(<VolumeIndicator level={0.5} />)
    const loud = container.querySelector("svg")?.getAttribute("class")
    // Act
    rerender(<VolumeIndicator level={0} />)
    // Assert
    expect(container.querySelector("svg")?.getAttribute("class")).not.toBe(loud)
    expect(screen.getByText("0%")).toBeInTheDocument()
    rerender(<VolumeIndicator level={null} />)
    expect(screen.queryByRole("status")).toBeNull()
  })

  it("T-ui-volind-03 [External] error state shows the error message itself, not a percent", () => {
    // Arrange + Act
    render(<VolumeIndicator level={null} system error="시스템 음량을 조정할 수 없습니다." />)
    // Assert
    expect(screen.getByRole("status")).toHaveTextContent("시스템 음량을 조정할 수 없습니다.")
    expect(screen.queryByText(/%/)).toBeNull()
  })
})

describe("MediaControls — wheel over the volume slider / mute button", () => {
  beforeEach(() => {
    stubLayout()
    vi.useFakeTimers({ toFake: ["Date"] })
  })
  afterEach(() => vi.useRealTimers())

  function setup() {
    const volumeBy = vi.fn()
    const systemVolumeBy = vi.fn()
    const stage = document.createElement("div")
    document.body.append(stage)
    function Harness() {
      const notches = useVolumeNotches(volumeBy, systemVolumeBy)
      useVolumeWheel(stage, notches)
      return <MediaControls state={STATE} fallbackDurationMs={null} frameStep={false} onTogglePlay={vi.fn()} onSeek={vi.fn()} onStep={vi.fn()} onRate={vi.fn()} onVolume={vi.fn()} onToggleMute={vi.fn()} onVolumeNotches={notches} />
    }
    render(<Harness />)
    return { volumeBy, systemVolumeBy, stage, slider: screen.getByRole("slider", { name: "음량" }), mute: screen.getByRole("button", { name: "음소거" }) }
  }
  const steps = (fn: ReturnType<typeof vi.fn>) => fn.mock.calls.map((c) => Math.round(c[0] / 0.05))

  it("T-ui-volwheel-12 [Happy] wheel up/down over the slider and over the mute button = +5% / -5% per notch, default prevented (no scroll)", () => {
    // Arrange
    const { volumeBy, systemVolumeBy, slider, mute } = setup()
    // Act
    const up = fireEvent.wheel(slider, { deltaY: -100 })
    const down = fireEvent.wheel(slider, { deltaY: 100 })
    fireEvent.wheel(mute, { deltaY: -200 })
    // Assert
    expect(up).toBe(false)
    expect(down).toBe(false)
    expect(steps(volumeBy)).toEqual([1, -1, 2])
    expect(systemVolumeBy).not.toHaveBeenCalled()
  })

  it("T-ui-volwheel-13 [Business rule] Shift+wheel over the slider = system volume (deltaY and Chromium's deltaX form); app volume untouched", () => {
    // Arrange
    const { volumeBy, systemVolumeBy, slider } = setup()
    // Act
    fireEvent.wheel(slider, { deltaY: -100, shiftKey: true })
    fireEvent.wheel(slider, { deltaX: 100, deltaY: 0, shiftKey: true })
    // Assert
    expect(steps(systemVolumeBy)).toEqual([1, -1])
    expect(volumeBy).not.toHaveBeenCalled()
  })

  it("T-ui-volwheel-14 [Business rule] the control-bar wheel and the stage wheel share ONE accumulator (0.6 + 0.6 notch = one step)", () => {
    // Arrange
    const { volumeBy, stage, slider } = setup()
    // Act
    fireEvent.wheel(slider, { deltaY: -60 })
    fireEvent.wheel(stage, { deltaY: -60 })
    // Assert
    expect(volumeBy).toHaveBeenCalledTimes(1)
    expect(volumeBy.mock.calls[0][0]).toBeCloseTo(0.05, 9)
  })

  it("T-ui-volwheel-15 [Boundary] wheel does not move focus (player area is focus-excluded); deltaY 0 and wheel elsewhere on the bar are ignored", () => {
    // Arrange
    const { volumeBy, slider } = setup()
    const outside = document.createElement("input")
    document.body.append(outside)
    outside.focus()
    // Act
    fireEvent.wheel(slider, { deltaY: -100 })
    fireEvent.wheel(slider, { deltaY: 0 })
    const elsewhere = fireEvent.wheel(screen.getByRole("slider", { name: "재생 위치" }), { deltaY: -100 })
    // Assert
    expect(document.activeElement).toBe(outside)
    expect(volumeBy).toHaveBeenCalledTimes(1)
    expect(elsewhere).toBe(true)
  })
})

describe("players — wheel over the volume slider", () => {
  it("T-ui-volwheel-17 [Happy] HTML player: wheel over the volume slider changes video.volume by 5% and shows the level on the stage; Shift steps the system volume", async () => {
    // Arrange
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
    const slider = await screen.findByRole("slider", { name: "음량" })
    // Act
    const prevented = !fireEvent.wheel(slider, { deltaY: -100 })
    // Assert
    expect(prevented).toBe(true)
    expect(video.volume).toBeCloseTo(0.55, 6)
    expect(await screen.findByText("음량 55%")).toBeInTheDocument()
    // Act: Shift
    fireEvent.wheel(slider, { deltaY: -100, shiftKey: true })
    // Assert
    expect(step).toHaveBeenCalledTimes(1)
    expect(video.volume).toBeCloseTo(0.55, 6)
    expect(await screen.findByText("시스템 음량 45%")).toBeInTheDocument()
  })

  it("T-ui-volwheel-18 [Happy] built-in player player: wheel over the volume slider calls playerSetVolume (-5%) and Shift calls the system volume", async () => {
    // Arrange
    const t = await mountMpv()
    const step = vi.spyOn(t.api, "systemVolumeStep").mockResolvedValue(0.6)
    await t.ready()
    t.setVolume.mockClear()
    const slider = screen.getByRole("slider", { name: "음량" })
    // Act
    fireEvent.wheel(slider, { deltaY: 100 })
    // Assert
    expect(t.setVolume).toHaveBeenCalledTimes(1)
    expect(t.setVolume.mock.calls[0][0]).toBe(95)
    // Act: Shift
    fireEvent.wheel(slider, { deltaY: -100, shiftKey: true })
    // Assert
    expect(step).toHaveBeenCalledTimes(1)
    expect(step.mock.calls[0][0]).toBeCloseTo(0.05, 9)
    expect(t.setVolume).toHaveBeenCalledTimes(1)
  })
})
