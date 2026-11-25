// Categories: Happy (T-ui-scancap-01 default + save) / Boundary (02 1 and 8 accepted) / Validation (03 0, 9, empty rejected inline) / External (04 server rejection shown inline)
import { waitFor, within } from "@testing-library/react"
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
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const api = createMockApi({ count: 20 })
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, "관리 폴더")
  const field = await within(page).findByLabelText("동시 스캔 수 (최대)")
  return { api, update, user, page, field }
}

describe("settings — 동시 스캔 수 (maxConcurrentScans)", () => {
  it("T-ui-scancap-01 [Happy] shows the default 3 in the scan section and saves a new value on blur", async () => {
    // Arrange
    const { api, update, user, page, field } = await setup()
    expect(field).toHaveValue(3)
    expect(within(page).getByRole("region", { name: "스캔" })).toContainElement(field)
    // Act
    await user.clear(field)
    await user.type(field, "5")
    await user.tab()
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ maxConcurrentScans: 5 }))
    expect((await api.getSettings()).maxConcurrentScans).toBe(5)
  })

  it.each([1, 8])("T-ui-scancap-02 [Boundary] the limit %i is accepted", async (n) => {
    // Arrange
    const { update, user, field } = await setup()
    // Act
    await user.clear(field)
    await user.type(field, String(n))
    await user.tab()
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ maxConcurrentScans: n }))
  })

  it.each(["0", "9", ""])("T-ui-scancap-03 [Validation] %j is rejected inline (aria-invalid + described) and nothing is saved", async (text) => {
    // Arrange
    const { update, user, page, field } = await setup()
    // Act
    await user.clear(field)
    if (text !== "") await user.type(field, text)
    await user.tab()
    // Assert
    const error = await within(page).findByText("1 이상 8 이하의 정수를 입력하세요.")
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(field).toHaveAttribute("aria-describedby", error.id)
    expect(update).not.toHaveBeenCalled()
  })

  it("T-ui-scancap-04 [External] a server rejection is shown inline and the field keeps the typed value", async () => {
    // Arrange
    const { api, user, page, field } = await setup()
    vi.spyOn(api, "updateSettings").mockRejectedValue({ code: "invalid_input", message: "invalid input: maxConcurrentScans" })
    // Act
    await user.clear(field)
    await user.type(field, "4")
    await user.tab()
    // Assert
    expect(await within(page).findByText("입력값이 올바르지 않습니다.")).toBeInTheDocument()
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(field).toHaveValue(4)
  })
})
