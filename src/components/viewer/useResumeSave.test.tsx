// Categories: Happy (pause / interval / unmount / seek) / Boundary (armed gate, rounding, unknown length) / Business rule (disabled gate, no save before restore) / External (save failure swallowed)
import { act, renderHook } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { RESUME_SAVE_INTERVAL_MS, RESUME_SEEK_SETTLE_MS, useResumeSave } from "./useResumeSave"

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

interface Props {
  enabled?: boolean
  armed?: boolean
  playing?: boolean
  positionMs?: number
  durationMs?: number | null
  seekTick?: number
}

function setup(initial: Props = {}) {
  const api: UiApi = createMockApi({ count: 5 })
  const save = vi.spyOn(api, "resumeSave").mockResolvedValue("saved")
  const wrapper = ({ children }: { children: ReactNode }) => <ApiProvider api={api}>{children}</ApiProvider>
  const base = { assetId: 1, enabled: true, armed: true, playing: false, positionMs: 61_234.6, durationMs: 600_000, seekTick: 0 }
  const hook = renderHook((p: Props) => useResumeSave({ ...base, ...p }), { wrapper, initialProps: initial })
  return { save, ...hook }
}

describe("useResumeSave", () => {
  it("T-ui-resume-01 [Happy] playing → paused saves the rounded position once with the duration", () => {
    // Arrange
    const t = setup({ playing: true })
    // Act
    t.rerender({ playing: false })
    // Assert
    expect(t.save).toHaveBeenCalledTimes(1)
    expect(t.save).toHaveBeenCalledWith(1, 61_235, 600_000)
  })

  it("T-ui-resume-02 [Happy] while playing it saves every 10s (and not while paused)", () => {
    // Arrange
    const t = setup({ playing: true })
    // Act + Assert: 10초 전에는 저장 없음
    act(() => void vi.advanceTimersByTime(RESUME_SAVE_INTERVAL_MS - 1))
    expect(t.save).not.toHaveBeenCalled()
    act(() => void vi.advanceTimersByTime(1))
    expect(t.save).toHaveBeenCalledTimes(1)
    act(() => void vi.advanceTimersByTime(RESUME_SAVE_INTERVAL_MS))
    expect(t.save).toHaveBeenCalledTimes(2)
    // 일시정지: 전이 저장 1회 + 이후 주기 저장 없음
    t.rerender({ playing: false })
    expect(t.save).toHaveBeenCalledTimes(3)
    act(() => void vi.advanceTimersByTime(RESUME_SAVE_INTERVAL_MS * 3))
    expect(t.save).toHaveBeenCalledTimes(3)
  })

  it("T-ui-resume-03 [Happy] unmount (item switch / close) saves the latest position", () => {
    // Arrange
    const t = setup({ playing: true, positionMs: 10_000 })
    t.rerender({ playing: true, positionMs: 95_000 })
    // Act
    t.unmount()
    // Assert
    expect(t.save).toHaveBeenCalledWith(1, 95_000, 600_000)
  })

  it("T-ui-resume-04 [Happy] a seek settles for 600ms then saves once — repeated seeks save only the last", () => {
    // Arrange
    const t = setup({ playing: false, positionMs: 20_000 })
    // Act
    t.rerender({ seekTick: 1, positionMs: 30_000 })
    act(() => void vi.advanceTimersByTime(RESUME_SEEK_SETTLE_MS - 100))
    t.rerender({ seekTick: 2, positionMs: 40_000 })
    act(() => void vi.advanceTimersByTime(RESUME_SEEK_SETTLE_MS - 1))
    expect(t.save).not.toHaveBeenCalled()
    act(() => void vi.advanceTimersByTime(1))
    // Assert
    expect(t.save).toHaveBeenCalledTimes(1)
    expect(t.save).toHaveBeenCalledWith(1, 40_000, 600_000)
  })

  it("T-ui-resume-05 [Business rule] nothing is saved before armed (no bogus 0 position from the initial open), even on unmount", () => {
    // Arrange
    const t = setup({ armed: false, playing: true, positionMs: 0 })
    // Act
    act(() => void vi.advanceTimersByTime(RESUME_SAVE_INTERVAL_MS * 2))
    t.rerender({ armed: false, playing: false })
    t.unmount()
    // Assert
    expect(t.save).not.toHaveBeenCalled()
  })

  it("T-ui-resume-06 [Business rule] disabled → no save on any trigger (gate OFF); enabled → saves (gate ON)", () => {
    // Arrange
    const off = setup({ enabled: false, playing: true })
    // Act
    act(() => void vi.advanceTimersByTime(RESUME_SAVE_INTERVAL_MS))
    off.rerender({ enabled: false, playing: false })
    off.unmount()
    // Assert
    expect(off.save).not.toHaveBeenCalled()
    const on = setup({ enabled: true, playing: true })
    act(() => void vi.advanceTimersByTime(RESUME_SAVE_INTERVAL_MS))
    expect(on.save).toHaveBeenCalledTimes(1)
  })

  it("T-ui-resume-07 [Boundary] once armed it stays armed — pause right after a lookup-failure gate still saves", () => {
    // Arrange: armed 가 잠깐 true 였다가 (재생 시작) 일시정지에서 조건이 다시 false 가 돼도 저장된다
    const t = setup({ armed: true, playing: true })
    // Act
    t.rerender({ armed: false, playing: false })
    // Assert
    expect(t.save).toHaveBeenCalledTimes(1)
  })

  it("T-ui-resume-08 [Boundary] unknown / zero length sends durationMs null; negative or non-finite position is clamped / skipped", () => {
    // Arrange
    const t = setup({ playing: true, durationMs: null, positionMs: -5 })
    // Act
    t.rerender({ playing: false, durationMs: null, positionMs: -5 })
    // Assert
    expect(t.save).toHaveBeenLastCalledWith(1, 0, null)
    t.rerender({ playing: true, durationMs: 0, positionMs: Number.NaN })
    t.rerender({ playing: false, durationMs: 0, positionMs: Number.NaN })
    expect(t.save).toHaveBeenCalledTimes(1) // NaN 위치는 보내지 않는다
  })

  it("T-ui-resume-09 [External] a rejected save is swallowed (playback is unaffected)", async () => {
    // Arrange
    const t = setup({ playing: true })
    t.save.mockRejectedValue({ code: "not_found", message: "asset not found" })
    // Act + Assert: 예외·미처리 거부 없이 지나간다
    expect(() => t.rerender({ playing: false })).not.toThrow()
    await act(async () => void (await Promise.resolve()))
    expect(t.save).toHaveBeenCalledTimes(1)
  })

  it("T-ui-resume-10 [Happy] pagehide and beforeunload save best-effort", () => {
    // Arrange
    const t = setup({ playing: true, positionMs: 70_000 })
    // Act
    window.dispatchEvent(new Event("pagehide"))
    window.dispatchEvent(new Event("beforeunload"))
    // Assert
    expect(t.save).toHaveBeenCalledTimes(2)
    expect(t.save).toHaveBeenCalledWith(1, 70_000, 600_000)
  })
})
