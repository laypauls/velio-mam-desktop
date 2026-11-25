// Categories: Happy (01 02) / Boundary (03 none = daily, 04 toggle off) / Business rule (02 saved order)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openFolderSettings } from "@/test/folderDialog"
import { openSettingsPage } from "@/test/settings"

type User = ReturnType<typeof userEvent.setup>

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function openSchedule() {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const user = userEvent.setup()
  const api = createMockApi({ count: 10 })
  renderApp(api)
  const settings = await openSettingsPage(user, "관리 폴더")
  const dialog = await openFolderSettings(user, settings, "D:\\Photos")
  await waitFor(() => expect(within(dialog).getByRole("combobox", { name: "방식" })).toBeEnabled())
  await user.click(within(dialog).getByRole("combobox", { name: "일정" }))
  await user.click(await screen.findByRole("option", { name: "요일·시각" }))
  return { user, api, dialog }
}

const day = (dialog: HTMLElement, name: string) => within(dialog).getByRole("button", { name })
const save = (user: User, dialog: HTMLElement) => user.click(within(dialog).getByRole("button", { name: "저장" }))

describe("folder settings dialog — weekday selection", () => {
  it("T-ui-weekday-01 [Happy] clicking 월 and 수 marks them pressed; the rest stay unpressed", async () => {
    // Arrange
    const { user, dialog } = await openSchedule()
    expect(day(dialog, "월")).toHaveAttribute("aria-pressed", "false")
    // Act
    await user.click(day(dialog, "수"))
    await user.click(day(dialog, "월"))
    // Assert
    expect(day(dialog, "월")).toHaveAttribute("aria-pressed", "true")
    expect(day(dialog, "수")).toHaveAttribute("aria-pressed", "true")
    expect(day(dialog, "화")).toHaveAttribute("aria-pressed", "false")
    expect(day(dialog, "일")).toHaveAttribute("aria-pressed", "false")
  })

  it("T-ui-weekday-02 [Business rule] saving sends the selected days in Mon→Sun order", async () => {
    // Arrange
    const { user, api, dialog } = await openSchedule()
    const spy = vi.spyOn(api, "setFolderRescanPolicy")
    // Act
    await user.click(day(dialog, "수"))
    await user.click(day(dialog, "월"))
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, { trigger: { kind: "schedule", days: ["mon", "wed"], at: "03:00" }, window: null, onStart: false })
  })

  it("T-ui-weekday-03 [Boundary] selecting no weekday saves an empty list (daily)", async () => {
    // Arrange
    const { user, api, dialog } = await openSchedule()
    const spy = vi.spyOn(api, "setFolderRescanPolicy")
    // Act
    await save(user, dialog)
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(spy).toHaveBeenCalledWith(1, { trigger: { kind: "schedule", days: [], at: "03:00" }, window: null, onStart: false })
  })

  it("T-ui-weekday-04 [Boundary] clicking a pressed weekday again unpresses it", async () => {
    // Arrange
    const { user, api, dialog } = await openSchedule()
    const spy = vi.spyOn(api, "setFolderRescanPolicy")
    await user.click(day(dialog, "월"))
    await user.click(day(dialog, "수"))
    // Act
    await user.click(day(dialog, "월"))
    // Assert
    expect(day(dialog, "월")).toHaveAttribute("aria-pressed", "false")
    expect(day(dialog, "수")).toHaveAttribute("aria-pressed", "true")
    await save(user, dialog)
    await waitFor(() => expect(spy).toHaveBeenCalledTimes(1))
    expect(spy).toHaveBeenCalledWith(1, { trigger: { kind: "schedule", days: ["wed"], at: "03:00" }, window: null, onStart: false })
  })
})
