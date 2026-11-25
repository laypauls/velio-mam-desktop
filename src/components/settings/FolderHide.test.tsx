// Categories: Happy (03) / External (04)
import { waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

const PATH = "D:\\Photos"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup() {
  stubLayout()
  const user = userEvent.setup()
  const api = createMockApi({ count: 10 })
  renderApp(api)
  const settings = await openSettingsPage(user, "관리 폴더")
  const button = await within(settings).findByRole("button", { name: `${PATH} 숨기기` })
  return { user, api, settings, button }
}

describe("hide managed folder (settings)", () => {
  it("T-ui-hide-03 [Happy] toggle calls setFolderHidden, label flips and badge appears, then unhide restores", async () => {
    // Arrange
    const { user, api, settings, button } = await setup()
    const spy = vi.spyOn(api, "setFolderHidden")
    // Act
    await user.click(button)
    // Assert
    const unhide = await within(settings).findByRole("button", { name: `${PATH} 숨김 해제` })
    expect(spy).toHaveBeenCalledWith(1, true)
    expect(within(settings).getByRole("img", { name: "숨김" })).toBeInTheDocument() // 텍스트 배지 대신 EyeOff 아이콘
    // Act (idempotent reverse)
    await user.click(unhide)
    // Assert
    expect(await within(settings).findByRole("button", { name: `${PATH} 숨기기` })).toBeInTheDocument()
    expect(spy).toHaveBeenLastCalledWith(1, false)
    expect(within(settings).queryByRole("img", { name: "숨김" })).not.toBeInTheDocument()
  })

  it("T-ui-hide-04 [External] failure shows an inline alert and keeps the visible state", async () => {
    // Arrange
    const { user, api, settings, button } = await setup()
    vi.spyOn(api, "setFolderHidden").mockRejectedValue({ code: "not_found", message: "x" })
    // Act
    await user.click(button)
    // Assert
    expect(await within(settings).findByRole("alert")).toHaveTextContent("대상을 찾을 수 없습니다.")
    await waitFor(() => expect(within(settings).getByRole("button", { name: `${PATH} 숨기기` })).toBeEnabled())
    expect(within(settings).queryByRole("img", { name: "숨김" })).not.toBeInTheDocument()
  })
})
