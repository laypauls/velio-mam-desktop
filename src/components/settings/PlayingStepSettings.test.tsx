// Categories: Happy (T-ui-step-10, 11) / Boundary (T-ui-step-12) / Validation (T-ui-step-09 mock range)
import { screen, waitFor, within } from "@testing-library/react"
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

async function setup(stored?: number) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const api = createMockApi({ count: 5 })
  if (stored !== undefined) await api.updateSettings({ playerPlayingStepSec: stored })
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, "플레이어")
  const select = await within(page).findByRole("combobox", { name: "재생 중 이동 폭" })
  return { api, update, user, select }
}

describe("settings > player > playing step", () => {
  it("T-ui-step-10 [Happy] shows 5초 by default with the 7 options, and picking one saves the number", async () => {
    // Arrange
    const { api, update, user, select } = await setup()
    expect(select).toHaveTextContent("5초")
    // Act
    await user.click(select)
    const options = await screen.findAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual(["1초", "2초", "3초", "5초", "10초", "15초", "30초"])
    await user.click(screen.getByRole("option", { name: "15초" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ playerPlayingStepSec: 15 }))
    expect((await api.getSettings()).playerPlayingStepSec).toBe(15)
    await waitFor(() => expect(select).toHaveTextContent("15초"))
  })

  it("T-ui-step-11 [Happy] the row shows the explanatory hint", async () => {
    await setup()
    expect(screen.getByText(/재생 중에 뒤로·앞으로 버튼/)).toBeInTheDocument()
  })

  it("T-ui-step-12 [Boundary] a stored value outside the preset list (7) is still shown and listed", async () => {
    // Arrange + Act
    const { user, select } = await setup(7)
    // Assert
    expect(select).toHaveTextContent("7초")
    await user.click(select)
    expect((await screen.findAllByRole("option")).map((o) => o.textContent)).toContain("7초")
  })

  it("T-ui-step-09 [Validation] the mock rejects out-of-range values like the backend (0, 61, 2.5)", async () => {
    const api = createMockApi({ count: 1 })
    for (const bad of [0, 61, 2.5]) await expect(api.updateSettings({ playerPlayingStepSec: bad })).rejects.toMatchObject({ code: "invalid_input" })
    expect((await api.getSettings()).playerPlayingStepSec).toBe(5)
  })
})
