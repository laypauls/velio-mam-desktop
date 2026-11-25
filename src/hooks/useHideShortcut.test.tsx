// Categories: Happy (default Esc) / Business rule (layers, editing, rebinding, disabled) / External failure (hide rejects)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useProtectStore, useSelectionStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(hideShortcut: string | null | undefined = undefined) {
  stubLayout()
  const media = stubMedia()
  const api = createMockApi({ count: 50 })
  await useStubbedDetail(api)
  if (hideShortcut !== undefined) await api.updateSettings({ hideShortcut })
  const hide = vi.spyOn(api, "hideMainWindow").mockResolvedValue(undefined)
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  return { api, hide, media, user }
}

describe("hide shortcut (window level)", () => {
  it("T-ui-hidekey-10 [Happy] default Esc pauses playing media and hides; the viewer stays open and the selection is kept", async () => {
    // Arrange
    const { hide, media, user } = await setup()
    act(() => useViewStore.getState().openPreview(1))
    act(() => useSelectionStore.getState().selectAll([1, 2]))
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    await video.play()
    expect(media.state.paused).toBe(false)
    // Act
    await user.keyboard("{Escape}")
    // Assert
    expect(hide).toHaveBeenCalledTimes(1)
    expect(media.state.paused).toBe(true)
    expect(useViewStore.getState().previewId).toBe(1)
    expect([...useSelectionStore.getState().ids]).toEqual([1, 2])
  })

  it("T-ui-hidekey-11 [Happy] Esc on the focused gallery list hides without clearing the selection", async () => {
    // Arrange
    const { hide, user } = await setup()
    const list = await screen.findByRole("listbox", { name: "미디어 목록" })
    act(() => useSelectionStore.getState().selectAll([3]))
    act(() => list.focus())
    // Act
    await user.keyboard("{Escape}")
    // Assert
    expect(hide).toHaveBeenCalledTimes(1)
    expect([...useSelectionStore.getState().ids]).toEqual([3])
  })

  it("T-ui-hidekey-12 [Business rule] Esc on the settings page hides and keeps settings open", async () => {
    // Arrange
    const { hide, user } = await setup()
    await openSettingsPage(user, "탐색·표시")
    // Act
    await user.keyboard("{Escape}")
    // Assert
    expect(hide).toHaveBeenCalledTimes(1)
    expect(useProtectStore.getState().settingsOpen).toBe(true)
    expect(screen.getByRole("navigation", { name: "설정 메뉴" })).toBeInTheDocument()
  })

  it("T-ui-hidekey-13 [Business rule] while a dialog is open Esc only closes the dialog; typing in an input is not intercepted", async () => {
    // Arrange
    const { hide, user } = await setup()
    const main = await openSettingsPage(user, "관리 폴더")
    await user.click((await within(main).findAllByRole("button", { name: / 설정$/ }))[0])
    await user.click(within(await screen.findByRole("dialog", { name: "관리 폴더 설정" })).getByRole("button", { name: "폴더 제거" }))
    await screen.findByRole("alertdialog")
    // Act: 겹친 대화상자 열림 — Esc 는 위의 것부터 하나씩 닫는다
    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument())
    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(hide).not.toHaveBeenCalled()
    // Act: 입력 요소 (설정을 닫고 검색창)
    await user.click(screen.getByRole("button", { name: "라이브러리로" }))
    const box = await screen.findByRole("combobox", { name: "검색" })
    await user.click(box)
    await user.keyboard("{Escape}")
    // Assert
    expect(hide).not.toHaveBeenCalled()
  })

  it("T-ui-hidekey-14 [Business rule] with Ctrl+Shift+H, Esc keeps closing the viewer and only the combo hides", async () => {
    // Arrange
    const { hide, user } = await setup("Ctrl+Shift+H")
    act(() => useViewStore.getState().openPreview(1))
    await screen.findByLabelText("clip.mp4", { selector: "video" })
    // Act + Assert: Esc 는 기존 동작(보기 영역 닫기)
    await user.keyboard("{Escape}")
    expect(useViewStore.getState().previewId).toBeNull()
    expect(hide).not.toHaveBeenCalled()
    // Act + Assert: 조합키
    await user.keyboard("{Control>}{Shift>}h{/Shift}{/Control}")
    expect(hide).toHaveBeenCalledTimes(1)
  })

  it("T-ui-hidekey-15 [Business rule, gate ON] disabled (null): nothing hides and Esc keeps its old behavior", async () => {
    // Arrange
    const { hide, user } = await setup(null)
    act(() => useViewStore.getState().openPreview(1))
    await screen.findByLabelText("clip.mp4", { selector: "video" })
    // Act
    await user.keyboard("{Escape}")
    await user.keyboard("{Control>}{Shift>}h{/Shift}{/Control}")
    // Assert
    expect(hide).not.toHaveBeenCalled()
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-hidekey-16 [External] a rejected hide command shows an error toast and does not crash", async () => {
    // Arrange
    const { api, hide, user } = await setup()
    hide.mockRejectedValue({ code: "internal", message: "boom" })
    expect(api).toBeDefined()
    // Act
    await user.keyboard("{Escape}")
    // Assert
    expect(await screen.findByText("알 수 없는 오류가 발생했습니다.")).toBeInTheDocument()
  })

  it("T-ui-hidekey-17 [Boundary] a held (repeating) key hides once and is swallowed", async () => {
    // Arrange
    const { hide, user } = await setup()
    act(() => useSelectionStore.getState().selectAll([3]))
    // Act
    await user.keyboard("{Escape>3/}")
    // Assert
    expect(hide).toHaveBeenCalledTimes(1)
    expect([...useSelectionStore.getState().ids]).toEqual([3])
  })
})
