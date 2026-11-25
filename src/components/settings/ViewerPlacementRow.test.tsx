// Categories: Happy (T-ui-settings-placement-01 default + save, 02 reflects stored value) / Validation (T-ui-settings-placement-03 invalid value rejected)
import { waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { SettingsPatch } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(settings: SettingsPatch = {}) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const api = createMockApi({ count: 50 })
  if (Object.keys(settings).length > 0) await api.updateSettings(settings)
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, "플레이어")
  await within(page).findByRole("region", { name: "플레이어" })
  const group = within(page).getByRole("radiogroup", { name: "플레이어 패널 위치" })
  return { api, update, user, group }
}

describe("settings > player > viewer placement", () => {
  it("T-ui-settings-placement-01 [Happy] defaults to 사이드바 왼쪽; choosing 갤러리 오른쪽 saves viewerPlacement 'right' and back saves 'left'", async () => {
    // Arrange
    const { update, user, group } = await setup()
    const right = within(group).getByRole("radio", { name: /갤러리 오른쪽/ })
    const left = within(group).getByRole("radio", { name: /사이드바 왼쪽/ })
    expect(left).toBeChecked()
    expect(right).not.toBeChecked()
    // Act
    await user.click(right)
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ viewerPlacement: "right" }))
    await waitFor(() => expect(right).toBeChecked())
    // Act
    await user.click(left)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ viewerPlacement: "left" }))
  })

  it("T-ui-settings-placement-02 [Happy] reflects an already stored 'right'", async () => {
    // Arrange + Act
    const { group } = await setup({ viewerPlacement: "right" })
    // Assert
    expect(within(group).getByRole("radio", { name: /갤러리 오른쪽/ })).toBeChecked()
    expect(within(group).getByRole("radio", { name: /사이드바 왼쪽/ })).not.toBeChecked()
  })

  it("T-ui-settings-placement-05 [Business rule] cards are ordered left → right and the 기본 배치 hint is on the left card only", async () => {
    // Arrange + Act
    const { group } = await setup()
    // Assert
    const radios = within(group).getAllByRole("radio")
    expect(radios).toHaveLength(2)
    expect(radios[0]).toHaveAccessibleName(/사이드바 왼쪽/)
    expect(radios[1]).toHaveAccessibleName(/갤러리 오른쪽/)
    expect(radios[0].closest("label")).toHaveTextContent(/기본 배치/)
    expect(radios[1].closest("label")).not.toHaveTextContent(/기본 배치/)
  })

  it("T-ui-settings-placement-04 [Happy] each card draws its layout preview (right/left order) hidden from screen readers", async () => {
    // Arrange + Act
    const { group } = await setup()
    // Assert
    const orderOf = (placement: string) => {
      const preview = group.querySelector(`[data-placement="${placement}"]`)
      expect(preview).not.toBeNull()
      expect(preview?.closest('[aria-hidden="true"]')).not.toBeNull()
      return Array.from(preview!.children).map((c) => c.getAttribute("data-part"))
    }
    expect(orderOf("right")).toEqual(["sidebar", "gallery", "player", "detail"])
    expect(orderOf("left")).toEqual(["player", "sidebar", "gallery", "detail"])
  })

  it("T-ui-settings-placement-03[Validation] the mock backend rejects a value other than right|left and keeps the stored one", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act + Assert
    await expect(api.updateSettings({ viewerPlacement: "center" as never })).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.getSettings()).resolves.toMatchObject({ viewerPlacement: "left" })
  })
})
