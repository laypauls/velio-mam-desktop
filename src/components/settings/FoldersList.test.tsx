// Categories: Business rule (01) / Happy (02)
import { within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("managed folder list (settings)", () => {
  it("T-ui-folders-list-01 [Business rule] list has no own height cap or scroll; the settings page scrolls instead", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 10 }))
    // Act
    const settings = await openSettingsPage(user, "관리 폴더")
    const table = await within(settings).findByRole("table", { name: "관리 폴더" })
    const wrapper = table.parentElement as HTMLElement
    // Assert
    for (const el of [table, wrapper]) {
      expect(el.className).not.toMatch(/(^|\s)max-h-/)
      expect(el.className).not.toMatch(/overflow-(x|y)?-?(auto|scroll)/)
    }
  })

  it("T-ui-folders-list-02 [Happy] one body row per folder plus a header row; each row has hide and settings buttons", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 10 })
    renderApp(api)
    const folders = await api.listFolders()
    // Act
    const settings = await openSettingsPage(user, "관리 폴더")
    const table = await within(settings).findByRole("table", { name: "관리 폴더" })
    // Assert: 머리글 1 + 폴더 수
    expect(within(table).getAllByRole("row")).toHaveLength(folders.length + 1)
    for (const f of folders) {
      expect(within(table).getByRole("button", { name: `${f.path} 설정` })).toBeInTheDocument()
      expect(within(table).getByRole("button", { name: new RegExp(`^${f.path.replace(/\\/g, "\\\\")} 숨기기$`) })).toBeInTheDocument()
    }
  })
})
