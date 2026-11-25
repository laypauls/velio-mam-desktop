// Categories: Happy (01) / External (02, 03) / Business rule (04)
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

async function setup() {
  stubLayout()
  const user = userEvent.setup()
  const api = createMockApi({ count: 10, backups: [] })
  const spy = vi.spyOn(api, "openBackupDir")
  renderApp(api)
  const settings = await openSettingsPage(user, "데이터·백업")
  const button = await within(settings).findByRole("button", { name: "탐색기에서 열기" })
  return { user, spy, settings, button }
}

describe("backup dir open-in-explorer button", () => {
  it("T-ui-backupdir-01 [Happy] click calls openBackupDir once", async () => {
    // Arrange
    const { user, spy, settings, button } = await setup()
    // Act
    await user.click(button)
    // Assert
    expect(spy).toHaveBeenCalledTimes(1)
    expect(within(settings).queryByRole("alert")).toBeNull()
  })

  it("T-ui-backupdir-02 [External] not_found shows the dedicated inline message", async () => {
    // Arrange
    const { user, spy, settings, button } = await setup()
    spy.mockRejectedValue({ code: "not_found", message: "missing" })
    // Act
    await user.click(button)
    // Assert
    const alert = await within(settings).findByRole("alert")
    expect(alert.textContent).toBe("백업 폴더가 없습니다. 백업을 한 번 실행하면 만들어집니다.")
  })

  it("T-ui-backupdir-03 [External] other errors show the generic error text", async () => {
    // Arrange
    const { user, spy, settings, button } = await setup()
    spy.mockRejectedValue({ code: "io", message: "launch failed" })
    // Act
    await user.click(button)
    // Assert
    const alert = await within(settings).findByRole("alert")
    expect(alert.textContent).toBe("파일 입출력 중 오류가 발생했습니다.")
  })

  it("T-ui-backupdir-04 [Business rule] error is cleared on the next successful click", async () => {
    // Arrange
    const { user, spy, settings, button } = await setup()
    spy.mockRejectedValueOnce({ code: "not_found", message: "missing" })
    await user.click(button)
    await within(settings).findByRole("alert")
    // Act
    await user.click(button)
    // Assert
    expect(spy).toHaveBeenCalledTimes(2)
    expect(within(settings).queryByRole("alert")).toBeNull()
  })
})
