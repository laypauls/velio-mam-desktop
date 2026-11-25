// Categories: Happy (T-ui-settings-sub-01 switch, languages) / Validation (T-ui-settings-sub-01 __off__ sentinel → null)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup() {
  stubLayout()
  const api = createMockApi({ count: 50 })
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const dialog = await openSettingsPage(user, "플레이어")
  await within(dialog).findByRole("region", { name: "플레이어" })
  return { api, update, user, dialog }
}

describe("settings > player > subtitles", () => {
  it("T-ui-settings-sub-01 [Happy] 자막 표시 defaults ON and saves { subtitleVisible } OFF then ON", async () => {
    // Arrange
    const { update, user, dialog, api } = await setup()
    const sw = within(dialog).getByRole("switch", { name: "자막 표시" })
    expect(sw).toBeChecked()
    // Act + Assert
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ subtitleVisible: false }))
    await waitFor(() => expect(sw).not.toBeChecked())
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ subtitleVisible: true }))
    expect((await api.getSettings()).subtitleVisible).toBe(true)
  })

  it("T-ui-settings-sub-01 [Validation] 자막 기본 언어 defaults to 한국어; each language saves its code; 자막 끄기 renders as a sentinel and saves null", async () => {
    // Arrange
    const { update, user, dialog, api } = await setup()
    const select = within(dialog).getByRole("combobox", { name: "자막 기본 언어" })
    expect(select).toHaveTextContent("한국어")
    // Act + Assert: 4개 언어
    for (const [label, code] of [["영어", "en"], ["일본어", "ja"], ["중국어", "zh"], ["한국어", "ko"]] as const) {
      await user.click(select)
      await user.click(await screen.findByRole("option", { name: label }))
      await waitFor(() => expect(update).toHaveBeenLastCalledWith({ subtitleDefaultLanguage: code }))
      await waitFor(() => expect(select).toHaveTextContent(label))
    }
    // Act: 자막 끄기 (옵션 값은 빈 문자열이 아니다)
    await user.click(select)
    const off = await screen.findByRole("option", { name: "자막 끄기" })
    await user.click(off)
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ subtitleDefaultLanguage: null }))
    await waitFor(() => expect(select).toHaveTextContent("자막 끄기"))
    expect((await api.getSettings()).subtitleDefaultLanguage).toBeNull()
    // Act: 다시 한국어
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "한국어" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ subtitleDefaultLanguage: "ko" }))
  })
})
