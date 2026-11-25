// Categories: Happy / Boundary (not ready, deltaY 0, deltaMode, fractional) / Business rule (cleanup, shared accumulator)
import { fireEvent, renderHook } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { useVolumeWheel } from "./useVolumeWheel"
import { WHEEL_IDLE_MS, createNotchAccumulator, useVolumeNotches, wheelNotches } from "./wheelNotches"

describe("useVolumeWheel", () => {
  it("T-ui-volwheel-04 [Boundary] stage not ready (null): wheel on a would-be stage does nothing until it is provided", () => {
    // Arrange
    const stage = document.createElement("div")
    const onDelta = vi.fn()
    const { rerender } = renderHook(({ s }: { s: HTMLElement | null }) => useVolumeWheel(s, onDelta), { initialProps: { s: null as HTMLElement | null } })
    // Act
    const notReady = fireEvent.wheel(stage, { deltaY: -1 })
    rerender({ s: stage })
    fireEvent.wheel(stage, { deltaY: -1 })
    // Assert
    expect(notReady).toBe(true) // preventDefault 되지 않음
    expect(onDelta).toHaveBeenCalledTimes(1)
  })

  it("T-ui-volwheel-05 [Happy] one pixel-mode notch (deltaY ±100) = ±1 notch, up positive, deltaY 0 ignored; listener removed on unmount", () => {
    // Arrange
    const stage = document.createElement("div")
    const onDelta = vi.fn()
    const { unmount } = renderHook(() => useVolumeWheel(stage, onDelta))
    // Act
    fireEvent.wheel(stage, { deltaY: -100 })
    fireEvent.wheel(stage, { deltaY: 100 })
    fireEvent.wheel(stage, { deltaY: 0 })
    unmount()
    fireEvent.wheel(stage, { deltaY: -100 })
    // Assert
    expect(onDelta.mock.calls).toEqual([[1, false], [-1, false]])
  })
})

describe("useVolumeWheel + useVolumeNotches (precision touchpad / deltaMode)", () => {
  const setup = () => {
    const stage = document.createElement("div")
    const volumeBy = vi.fn()
    renderHook(() => useVolumeWheel(stage, useVolumeNotches(volumeBy)))
    return { stage, volumeBy }
  }
  beforeEach(() => vi.useFakeTimers({ toFake: ["Date"] }))
  afterEach(() => vi.useRealTimers())

  it("T-ui-volwheel-06 [Boundary] many small pixel deltas accumulate: ten events of 10px = one notch, not ten", () => {
    // Arrange
    const { stage, volumeBy } = setup()
    // Act
    for (let i = 0; i < 9; i++) fireEvent.wheel(stage, { deltaY: -10 })
    const before = volumeBy.mock.calls.length
    fireEvent.wheel(stage, { deltaY: -10 })
    // Assert
    expect(before).toBe(0)
    expect(volumeBy).toHaveBeenCalledTimes(1)
    expect(volumeBy.mock.calls[0][0]).toBeCloseTo(0.05, 9)
  })

  it("T-ui-volwheel-07 [Business rule] a big pixel burst moves by its whole notches only (250px = 2 notches, remainder kept)", () => {
    // Arrange
    const { stage, volumeBy } = setup()
    // Act
    fireEvent.wheel(stage, { deltaY: 250 })
    fireEvent.wheel(stage, { deltaY: 50 })
    // Assert: 2.5 → -2 노치, 남은 0.5 + 0.5 → -1 노치
    expect(volumeBy.mock.calls.map((c) => Math.round(c[0] / 0.05))).toEqual([-2, -1])
  })

  it("T-ui-volwheel-08 [Boundary] deltaMode line (3 lines = 1 notch) and page (1 page = 1 notch); unknown mode is read as pixel", () => {
    // Arrange
    const { stage, volumeBy } = setup()
    // Act
    fireEvent.wheel(stage, { deltaY: -3, deltaMode: 1 })
    fireEvent.wheel(stage, { deltaY: -1, deltaMode: 2 })
    fireEvent.wheel(stage, { deltaY: -100, deltaMode: 9 })
    // Assert
    expect(volumeBy.mock.calls.map((c) => Math.round(c[0] / 0.05))).toEqual([1, 1, 1])
  })

  it("T-ui-volwheel-09 [Boundary] a direction change or an idle gap drops the remainder", () => {
    // Arrange
    const { stage, volumeBy } = setup()
    // Act 1: 위 60 → 아래 60 : 방향이 바뀌어 나머지가 버려지므로 0
    fireEvent.wheel(stage, { deltaY: -60 })
    fireEvent.wheel(stage, { deltaY: 60 })
    // Act 2: 오래 멈춘 뒤 아래 60 : 남은 -0.6 에 더해지지 않는다
    vi.setSystemTime(Date.now() + WHEEL_IDLE_MS + 1)
    fireEvent.wheel(stage, { deltaY: 60 })
    // Assert
    expect(volumeBy).not.toHaveBeenCalled()
    // Act 3: 곧바로 이어서 60 → -1.2 → 한 노치
    fireEvent.wheel(stage, { deltaY: 60 })
    expect(volumeBy).toHaveBeenCalledTimes(1)
  })

  it("T-ui-volwheel-10 [Validation] wheelNotches / accumulator ignore NaN and Infinity", () => {
    // Arrange
    const push = createNotchAccumulator(() => 0)
    // Act + Assert
    expect(push(Number.NaN)).toBe(0)
    expect(push(Number.POSITIVE_INFINITY)).toBe(0)
    expect(wheelNotches(-100, 0)).toBe(1)
    expect(wheelNotches(3, 1)).toBe(-1)
  })

  it("T-ui-volwheel-11 [Business rule] the web stage wheel and the native input path share ONE accumulator", () => {
    // Arrange: 같은 useVolumeNotches 결과를 두 입력 경로가 나눠 쓴다
    const stage = document.createElement("div")
    const volumeBy = vi.fn()
    const { result } = renderHook(() => {
      const onNotches = useVolumeNotches(volumeBy)
      useVolumeWheel(stage, onNotches)
      return onNotches
    })
    // Act: 웹 0.6 + 네이티브 0.6
    fireEvent.wheel(stage, { deltaY: -60 })
    result.current(0.6)
    // Assert
    expect(volumeBy).toHaveBeenCalledTimes(1)
    expect(volumeBy.mock.calls[0][0]).toBeCloseTo(0.05, 9)
  })
})

describe("Shift wheel = system volume", () => {
  const setup = () => {
    const stage = document.createElement("div")
    const volumeBy = vi.fn()
    const systemVolumeBy = vi.fn()
    renderHook(() => useVolumeWheel(stage, useVolumeNotches(volumeBy, systemVolumeBy)))
    return { stage, volumeBy, systemVolumeBy }
  }
  beforeEach(() => vi.useFakeTimers({ toFake: ["Date"] }))
  afterEach(() => vi.useRealTimers())

  it("T-ui-sysvol-wheel-01 [Happy] Shift+wheel (deltaY) steps the system volume ±5% per notch; plain wheel stays app volume", () => {
    // Arrange
    const { stage, volumeBy, systemVolumeBy } = setup()
    // Act
    fireEvent.wheel(stage, { deltaY: -200, shiftKey: true })
    fireEvent.wheel(stage, { deltaY: 100, shiftKey: true })
    // Assert
    expect(systemVolumeBy.mock.calls.map((c) => Math.round(c[0] / 0.05))).toEqual([2, -1])
    expect(volumeBy).not.toHaveBeenCalled()
    // Act: Shift 없이
    fireEvent.wheel(stage, { deltaY: -100 })
    expect(volumeBy).toHaveBeenCalledTimes(1)
    expect(systemVolumeBy).toHaveBeenCalledTimes(2)
  })

  it("T-ui-sysvol-wheel-02 [Boundary] Chromium reports Shift+wheel as deltaX: it is used when deltaY is 0, keeps the sign, and is prevented; a plain horizontal wheel is ignored", () => {
    // Arrange
    const { stage, volumeBy, systemVolumeBy } = setup()
    // Act
    const prevented = !fireEvent.wheel(stage, { deltaX: -100, deltaY: 0, shiftKey: true })
    fireEvent.wheel(stage, { deltaX: 100, deltaY: 0, shiftKey: true })
    const horizontalPlain = fireEvent.wheel(stage, { deltaX: 100, deltaY: 0 })
    // Assert
    expect(prevented).toBe(true)
    expect(systemVolumeBy.mock.calls.map((c) => Math.round(c[0] / 0.05))).toEqual([1, -1])
    expect(horizontalPlain).toBe(true)
    expect(volumeBy).not.toHaveBeenCalled()
  })

  it("T-ui-sysvol-wheel-03 [Concurrency] app and system accumulators are separate: a half notch on one never completes with a half notch on the other", () => {
    // Arrange
    const { stage, volumeBy, systemVolumeBy } = setup()
    // Act: 앱 0.6 → 시스템 0.6 → 앱 0.6 (같은 방향, 400ms 안)
    fireEvent.wheel(stage, { deltaY: -60 })
    fireEvent.wheel(stage, { deltaY: -60, shiftKey: true })
    fireEvent.wheel(stage, { deltaY: -60 })
    // Assert: 어느 쪽도 한 노치를 못 채웠다
    expect(volumeBy).not.toHaveBeenCalled()
    expect(systemVolumeBy).not.toHaveBeenCalled()
  })

  it("T-ui-sysvol-wheel-04 [Business rule] without a system handler Shift+wheel does nothing (never falls back to app volume)", () => {
    // Arrange
    const stage = document.createElement("div")
    const volumeBy = vi.fn()
    renderHook(() => useVolumeWheel(stage, useVolumeNotches(volumeBy)))
    // Act
    fireEvent.wheel(stage, { deltaY: -100, shiftKey: true })
    // Assert
    expect(volumeBy).not.toHaveBeenCalled()
  })
})
