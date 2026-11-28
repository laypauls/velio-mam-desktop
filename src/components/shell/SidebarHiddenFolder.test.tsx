// Categories: Happy (07) / Business rule (08, 09 view reset) / Boundary (12 all hidden)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useSearchStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

const PHOTOS = "D:\\Photos"
const VIDEOS = "D:\\Videos"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("sidebar and hidden managed folders", () => {
  it("T-ui-hide-07 [Happy] hidden folder is not listed in the sidebar tree", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 10 })
    await api.setFolderHidden(1, true)
    // Act
    renderApp(api, { section: "folders" })
    // Assert
    const nav = await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    expect(await within(nav).findByRole("button", { name: VIDEOS })).toBeInTheDocument()
    expect(within(nav).queryByRole("button", { name: PHOTOS })).not.toBeInTheDocument()
  })

  it("T-ui-hide-08 [Business rule] hiding the folder being viewed resets the view to all", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 10 }), { section: "folders" })
    const nav = await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    await user.click(await within(nav).findByRole("button", { name: PHOTOS }))
    expect(useSearchStore.getState().view).toMatchObject({ kind: "folder", folderId: 1 })
    // Act
    const settings = await openSettingsPage(user, "관리 폴더")
    await user.click(await within(settings).findByRole("button", { name: `${PHOTOS} 숨기기` }))
    // Assert
    await waitFor(() => expect(useSearchStore.getState().view).toEqual({ kind: "all" }))
  })

  it("T-ui-hide-09 [Business rule] view resets even while the sidebar is closed", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 10 }))
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    act(() => {
      useSearchStore.getState().setView({ kind: "folder", folderId: 1, relPath: null })
      useViewStore.getState().toggleSidebar() // 사용자 변경 — 설정 로드 뒤 저장값 채우기가 덮어쓰지 않는다
    })
    expect(screen.queryByRole("navigation", { name: "라이브러리 탐색" })).not.toBeInTheDocument()
    // Act
    const settings = await openSettingsPage(user, "관리 폴더")
    await user.click(await within(settings).findByRole("button", { name: `${PHOTOS} 숨기기` }))
    // Assert
    await waitFor(() => expect(useSearchStore.getState().view).toEqual({ kind: "all" }))
  })

  it("T-ui-hide-12 [Boundary] all folders hidden shows a distinct message, not the 'none registered' one", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 10 })
    for (const f of await api.listFolders()) await api.setFolderHidden(f.folderId, true)
    // Act
    renderApp(api, { section: "folders" })
    // Assert
    expect(await screen.findByText(/모든 관리 폴더가 숨겨져 있습니다/)).toBeInTheDocument()
    expect(screen.queryByText("등록된 관리 폴더가 없습니다.")).not.toBeInTheDocument()
  })
})
