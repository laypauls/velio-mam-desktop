// Categories: Happy (T-ui-vol-01) / Concurrency (T-ui-vol-02) / Time (T-ui-vol-03) / External (T-ui-vol-04) / Validation (T-ui-vol-05, 06) / Business rule (T-ui-vol-07)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render } from "@testing-library/react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { Settings } from "@/lib/ipc-contract"
import { resetAllStores, usePlayerVolumeStore } from "@/stores"
import { normalizePlayerVolume } from "@/stores/playerVolume"
import { PLAYER_VOLUME_SAVE_DELAY_MS, usePlayerVolumePersistence } from "./usePlayerVolumePersistence"

function Host() {
  usePlayerVolumePersistence()
  return null
}

beforeEach(() => {
  vi.useFakeTimers()
  resetAllStores()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

const STORED = { level: 35, muted: true }
const volumeOf = () => {
  const { level, muted } = usePlayerVolumeStore.getState()
  return { level, muted }
}

/** getSettings/updateSettings 를 즉시 해결되는 스파이로 바꾸고 훅만 렌더한다. `load: false` 면 getSettings 를 직접 풀어 준다 */
async function setup(stored: unknown = STORED, opts: { load?: boolean; updateFails?: boolean } = {}) {
  const api = createMockApi({ count: 1 })
  const base = await api.getSettings()
  const settings = { ...base, playerVolume: stored } as Settings
  let release: () => void = () => undefined
  vi.spyOn(api, "getSettings").mockImplementation(() =>
    opts.load === false ? new Promise((r) => (release = () => r(settings))) : Promise.resolve(settings)
  )
  const update = vi.spyOn(api, "updateSettings").mockImplementation(() =>
    opts.updateFails ? Promise.reject({ code: "io", message: "disk" }) : Promise.resolve(settings)
  )
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <Host />
      </QueryClientProvider>
    </ApiProvider>
  )
  await act(async () => {
    if (opts.load === false) return
    await vi.advanceTimersByTimeAsync(0)
  })
  return { update, release: () => act(async () => { release(); await vi.advanceTimersByTimeAsync(0) }) }
}

describe("usePlayerVolumePersistence", () => {
  it("T-ui-vol-01 [Happy] stored volume and mute hydrate the store once without triggering a save", async () => {
    // Arrange + Act
    const { update } = await setup()
    // Assert
    expect(volumeOf()).toEqual(STORED)
    expect(usePlayerVolumeStore.getState().rev).toBe(0)
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS * 2))
    expect(update).not.toHaveBeenCalled()
  })

  it("T-ui-vol-02 [Concurrency] a change made before settings load wins over the stored value and is saved", async () => {
    // Arrange: 설정이 아직 로드되지 않음
    const { update, release } = await setup(STORED, { load: false })
    // Act: 사용자가 먼저 음량을 바꾸고, 그 뒤 저장값 도착
    act(() => usePlayerVolumeStore.getState().setLevel(60))
    await release()
    // Assert: 저장값(35, muted)으로 덮이지 않는다
    expect(volumeOf()).toEqual({ level: 60, muted: false })
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS))
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith({ playerVolume: { level: 60, muted: false } })
  })

  it("T-ui-vol-03 [Time] saves once, 300ms after the last change (slider drag collapses into one write)", async () => {
    // Arrange
    const { update } = await setup({ level: 100, muted: false })
    // Act
    for (const l of [90, 80, 70, 60]) {
      act(() => usePlayerVolumeStore.getState().setLevel(l))
      await act(() => vi.advanceTimersByTimeAsync(100))
    }
    // Assert: 마지막 변경 뒤 299ms 까지는 저장 없음
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS - 101))
    expect(update).not.toHaveBeenCalled()
    await act(() => vi.advanceTimersByTimeAsync(1))
    expect(update).toHaveBeenCalledTimes(1)
    expect(update).toHaveBeenCalledWith({ playerVolume: { level: 60, muted: false } })
  })

  it("T-ui-vol-04 [External] a failed save shows an error toast and keeps the local state", async () => {
    // Arrange
    const toastError = vi.spyOn(toast, "error").mockReturnValue("t")
    const { update } = await setup({ level: 100, muted: false }, { updateFails: true })
    // Act
    act(() => usePlayerVolumeStore.getState().setMuted(true))
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS))
    // Assert
    expect(update).toHaveBeenCalledTimes(1)
    expect(toastError).toHaveBeenCalledWith("파일 입출력 중 오류가 발생했습니다.")
    expect(volumeOf()).toEqual({ level: 100, muted: true })
  })

  it("T-ui-vol-05 [Validation] broken stored values fall back field by field; level is clamped to 0..100 and rounded", async () => {
    // Arrange + Act
    await setup({ level: 250.4, muted: "yes" })
    // Assert
    expect(volumeOf()).toEqual({ level: 100, muted: false })
    expect(normalizePlayerVolume({ level: -5, muted: true })).toEqual({ level: 0, muted: true })
    expect(normalizePlayerVolume({ level: 49.6 }).level).toBe(50)
  })

  it("T-ui-vol-06 [Boundary] normalizePlayerVolume handles non-objects and NaN", () => {
    const defaults = { level: 100, muted: false }
    expect(normalizePlayerVolume(null)).toEqual(defaults)
    expect(normalizePlayerVolume("x")).toEqual(defaults)
    expect(normalizePlayerVolume({ level: Number.NaN })).toEqual(defaults)
    expect(normalizePlayerVolume({ level: "30" })).toEqual(defaults)
    expect(normalizePlayerVolume({ level: 0 }).level).toBe(0)
  })

  it("T-ui-vol-07 [Business rule] level 0 and mute are stored separately; unmuting keeps the level; equal values do not schedule a save", async () => {
    // Arrange
    const { update } = await setup({ level: 100, muted: false })
    // Act: 같은 값은 저장하지 않는다
    act(() => {
      usePlayerVolumeStore.getState().setLevel(100)
      usePlayerVolumeStore.getState().setMuted(false)
    })
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS * 2))
    expect(update).not.toHaveBeenCalled()
    // Act: 음소거 → 저장은 level 유지
    act(() => usePlayerVolumeStore.getState().setMuted(true))
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS))
    expect(update).toHaveBeenLastCalledWith({ playerVolume: { level: 100, muted: true } })
    // Act: 0 으로 내려도 음소거 상태는 따로, 올리면 음소거 해제
    act(() => usePlayerVolumeStore.getState().setLevel(0))
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS))
    expect(update).toHaveBeenLastCalledWith({ playerVolume: { level: 0, muted: true } })
    act(() => usePlayerVolumeStore.getState().setLevel(20))
    await act(() => vi.advanceTimersByTimeAsync(PLAYER_VOLUME_SAVE_DELAY_MS))
    expect(update).toHaveBeenLastCalledWith({ playerVolume: { level: 20, muted: false } })
  })
})
