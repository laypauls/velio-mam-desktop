// Categories: Happy (T-ui-vol-10, 12, 13, 14) / Business rule (T-ui-vol-11, 15) / Boundary (T-ui-vol-16)
import { act, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { resetAllStores, usePlayerVolumeStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { mountMpv } from "@/test/mpv"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** <video> 경로 — 영상을 열고, 파일 전환 후 새 요소를 찾는 도우미를 돌려준다 */
async function setupHtml(stored?: { level: number; muted: boolean }) {
  resetAllStores()
  stubLayout()
  stubMedia(100)
  const api = createMockApi({ count: 5 })
  await useStubbedDetail(api, { mediaType: "video", fileName: "clip.mp4", fps: 25 })
  if (stored) await api.updateSettings({ playerVolume: stored }) // 설정에 저장된 값 — 앱 셸이 불러와 채운다
  const user = userEvent.setup()
  renderApp(api)
  await screen.findByRole("listbox", { name: "미디어 목록" })
  act(() => useViewStore.getState().openPreview(1))
  const video = () => screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
  return { user, video, first: await video() }
}

describe("<video> volume memory", () => {
  it("T-ui-vol-10 [Happy] the volume survives opening another file", async () => {
    // Arrange
    const { user, video, first } = await setupHtml()
    // Act: 100% → 85%, 다른 파일 열기
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}")
    expect(first.volume).toBeCloseTo(0.85, 6)
    act(() => useViewStore.getState().openPreview(2))
    // Assert
    const second = await video()
    await waitFor(() => expect(second.volume).toBeCloseTo(0.85, 6))
    expect(second.muted).toBe(false)
    expect(usePlayerVolumeStore.getState().level).toBe(85)
  })

  it("T-ui-vol-11 [Business rule] mute survives a file switch and unmuting restores the previous level", async () => {
    // Arrange
    const { user, video } = await setupHtml()
    await user.keyboard("{ArrowDown}{ArrowDown}") // 90%
    // Act: 음소거 후 다른 파일
    await user.keyboard("m")
    act(() => useViewStore.getState().openPreview(2))
    const second = await video()
    // Assert: 새 파일도 음소거로 열리고 버튼은 해제 동작을 안내한다
    await waitFor(() => expect(second.muted).toBe(true))
    expect(screen.getByRole("button", { name: "음소거 해제" })).toHaveAttribute("aria-pressed", "true")
    // Act: 해제
    await user.click(screen.getByRole("button", { name: "음소거 해제" }))
    // Assert: 이전 level 로 복귀
    expect(second.muted).toBe(false)
    expect(second.volume).toBeCloseTo(0.9, 6)
    expect(usePlayerVolumeStore.getState()).toMatchObject({ level: 90, muted: false })
  })

  it("T-ui-vol-12 [Happy] a stored volume is applied to the element when the player opens", async () => {
    // Arrange + Act: 저장값이 먼저 채워진 상태로 열기
    const { first } = await setupHtml({ level: 40, muted: true })
    // Assert
    await waitFor(() => expect(first.muted).toBe(true))
    expect(first.volume).toBeCloseTo(0.4, 6)
  })

  it("T-ui-vol-16 [Boundary] level 0 mutes the element but keeps the store's mute flag false", async () => {
    // Arrange
    const { user, first } = await setupHtml()
    first.volume = 0.05
    // Act
    await user.keyboard("{ArrowDown}")
    // Assert
    expect(first.volume).toBe(0)
    expect(first.muted).toBe(true)
    expect(usePlayerVolumeStore.getState()).toMatchObject({ level: 0, muted: false })
  })
})

describe("built-in player volume memory", () => {
  it("T-ui-vol-13 [Happy] opening applies the stored level and mute through player_set_volume", async () => {
    // Arrange: 열기가 끝나기 전에 저장값이 들어온다
    const t = await mountMpv({ open: "pending" })
    act(() => usePlayerVolumeStore.getState().hydrate({ level: 35, muted: true }))
    // Act
    await waitFor(() => expect(t.open).toHaveBeenCalled())
    await t.finishOpen()
    await t.ready()
    // Assert
    await waitFor(() => expect(t.setVolume).toHaveBeenLastCalledWith(35, true))
    expect(t.setVolume).not.toHaveBeenCalledWith(100, false)
  })

  it("T-ui-vol-14 [Happy] the volume survives opening another file", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv()
    await t.ready()
    await waitFor(() => expect(t.setVolume).toHaveBeenLastCalledWith(100, false))
    // Act: 100% → 85%, 다른 파일 열기
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}")
    await waitFor(() => expect(t.setVolume).toHaveBeenLastCalledWith(85, false))
    t.setVolume.mockClear()
    act(() => useViewStore.getState().openPreview(2))
    await waitFor(() => expect(screen.getByRole("img", { name: "v2.mxf" })).toHaveAttribute("aria-busy", "false"))
    // Assert: 새 파일도 저장 음량으로 열리고, 100 으로 강제하지 않는다
    await waitFor(() => expect(t.setVolume).toHaveBeenLastCalledWith(85, false))
    expect(t.setVolume).not.toHaveBeenCalledWith(100, false)
  })

  it("T-ui-vol-15 [Business rule] mute survives a file switch and unmuting restores the previous level", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv()
    await t.ready()
    await user.keyboard("{ArrowDown}{ArrowDown}") // 90%
    await user.keyboard("m")
    await waitFor(() => expect(t.setVolume).toHaveBeenLastCalledWith(90, true))
    // Act: 다른 파일
    t.setVolume.mockClear()
    act(() => useViewStore.getState().openPreview(2))
    await waitFor(() => expect(screen.getByRole("img", { name: "v2.mxf" })).toHaveAttribute("aria-busy", "false"))
    // Assert: 음소거 유지
    await waitFor(() => expect(t.setVolume).toHaveBeenLastCalledWith(90, true))
    expect(screen.getByRole("button", { name: "음소거 해제" })).toHaveAttribute("aria-pressed", "true")
    // Act: 해제 → 이전 level
    await user.click(screen.getByRole("button", { name: "음소거 해제" }))
    expect(t.setVolume).toHaveBeenLastCalledWith(90, false)
  })
})
