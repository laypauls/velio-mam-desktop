// Categories: Happy (default Q, IME) / Business rule (layers, editable, disabled, closed viewer, hide wins) / Boundary (repeat)
import { act, fireEvent, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { Settings } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(patch: Partial<Settings> = {}, open = true) {
  stubLayout()
  stubMedia()
  const api = createMockApi({ count: 50 })
  await useStubbedDetail(api)
  await api.updateSettings(patch)
  const hide = vi.spyOn(api, "hideMainWindow").mockResolvedValue(undefined)
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  if (open) {
    act(() => useViewStore.getState().openPreview(1))
    await screen.findByLabelText("clip.mp4", { selector: "video" })
  }
  return { api, hide, user }
}

describe("close viewer shortcut", () => {
  it("T-ui-closekey-10 [Happy] default Q closes only the viewer (same path as the close button) and does not hide the app", async () => {
    // Arrange
    const { hide, user } = await setup()
    // Act
    await user.keyboard("q")
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
    expect(hide).not.toHaveBeenCalled()
  })

  it("T-ui-closekey-11 [Happy] Korean IME (key ㅂ, code KeyQ) closes the viewer", async () => {
    // Arrange
    await setup()
    // Act
    fireEvent.keyDown(document.body, { key: "ㅂ", code: "KeyQ" })
    // Assert
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-closekey-12 [Business rule] ignored in an editable target and while a dialog is open", async () => {
    // Arrange
    const { user } = await setup()
    const box = await screen.findByRole("combobox", { name: "검색" })
    // Act + Assert: 입력 요소
    await user.click(box)
    await user.keyboard("q")
    expect(useViewStore.getState().previewId).toBe(1)
    // Act + Assert: 열린 대화상자
    const dialog = document.createElement("div")
    dialog.setAttribute("role", "dialog")
    document.body.appendChild(dialog)
    fireEvent.keyDown(document.body, { key: "q", code: "KeyQ" })
    expect(useViewStore.getState().previewId).toBe(1)
    dialog.remove()
    fireEvent.keyDown(document.body, { key: "q", code: "KeyQ" })
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-closekey-13 [Business rule, gate ON] disabled (null) keeps the viewer open", async () => {
    // Arrange
    const { user } = await setup({ closeViewerShortcut: null })
    // Act
    await user.keyboard("q")
    // Assert
    expect(useViewStore.getState().previewId).toBe(1)
  })

  it("T-ui-closekey-14 [Business rule] with the viewer closed Q does nothing (no listener, no preventDefault)", async () => {
    // Arrange
    await setup({}, false)
    // Act
    const notPrevented = fireEvent.keyDown(document.body, { key: "q", code: "KeyQ" })
    // Assert
    expect(notPrevented).toBe(true)
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-closekey-15 [Boundary] a repeating key is swallowed without closing", async () => {
    // Arrange
    await setup()
    // Act
    const notPrevented = fireEvent.keyDown(document.body, { key: "q", code: "KeyQ", repeat: true })
    // Assert
    expect(notPrevented).toBe(false)
    expect(useViewStore.getState().previewId).toBe(1)
  })

  it("T-ui-closekey-16 [Business rule] a custom key replaces Q; Esc stays the viewer's own close key", async () => {
    // Arrange
    const { user } = await setup({ closeViewerShortcut: "F7", hideShortcut: null })
    // Act + Assert
    await user.keyboard("q")
    expect(useViewStore.getState().previewId).toBe(1)
    await user.keyboard("{F7}")
    expect(useViewStore.getState().previewId).toBeNull()
  })

  it("T-ui-closekey-17 [Concurrency] if both shortcuts were the same stored key, the hide shortcut wins and the viewer stays open", async () => {
    // Arrange
    const { hide, user } = await setup({ closeViewerShortcut: "F7", hideShortcut: "F7" })
    // Act
    await user.keyboard("{F7}")
    // Assert
    await waitFor(() => expect(hide).toHaveBeenCalledTimes(1))
    expect(useViewStore.getState().previewId).toBe(1)
  })
})

describe("hide shortcut physical key + mock contract", () => {
  it("T-ui-closekey-18 [Happy] a letter hide shortcut also fires from the Korean IME (key ㅂ, code KeyQ)", async () => {
    // Arrange
    const { hide } = await setup({ hideShortcut: "Q", closeViewerShortcut: null })
    // Act
    fireEvent.keyDown(document.body, { key: "ㅂ", code: "KeyQ" })
    // Assert
    await waitFor(() => expect(hide).toHaveBeenCalledTimes(1))
    expect(useViewStore.getState().previewId).toBe(1)
  })

  it("T-ui-closekey-19 [Validation] the mock defaults closeViewerShortcut to Q, accepts null and rejects a malformed value", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act + Assert
    expect((await api.getSettings()).closeViewerShortcut).toBe("Q")
    expect((await api.updateSettings({ closeViewerShortcut: null })).closeViewerShortcut).toBeNull()
    await expect(api.updateSettings({ closeViewerShortcut: "Shift+Ctrl+Q" })).rejects.toMatchObject({ code: "invalid_input" })
    expect((await api.getSettings()).closeViewerShortcut).toBeNull()
  })
})
