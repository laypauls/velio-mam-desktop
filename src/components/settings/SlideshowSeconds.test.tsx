// Categories: Happy + Validation + External (T-ui-set-ss-01)
import { fireEvent, screen, waitFor, within } from "@testing-library/react"
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

describe("settings > slideshow image seconds", () => {
  it("T-ui-set-ss-01 [Happy/Validation] defaults to 5, saves 1..60 on Enter, rejects 0 and 61 inline without saving", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openSettingsPage(user, "플레이어")
    const input = await within(dialog).findByRole("spinbutton", { name: "연속 재생 이미지 표시 시간" })
    expect(input).toHaveValue(5)
    // Act + Assert: 범위 밖은 저장하지 않고 인라인 오류
    for (const bad of ["0", "61"]) {
      fireEvent.change(input, { target: { value: bad } })
      fireEvent.keyDown(input, { key: "Enter" })
      expect(await within(dialog).findByText("1 이상 60 이하의 정수를 입력하세요.")).toBeInTheDocument()
      expect(input).toHaveAttribute("aria-invalid", "true")
    }
    expect(update).not.toHaveBeenCalled()
    // 경계값 저장
    fireEvent.change(input, { target: { value: "60" } })
    fireEvent.keyDown(input, { key: "Enter" })
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ slideshowImageSeconds: 60 }))
    fireEvent.change(input, { target: { value: "1" } })
    fireEvent.keyDown(input, { key: "Enter" })
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ slideshowImageSeconds: 1 }))
    expect((await api.getSettings()).slideshowImageSeconds).toBe(1)
    expect(screen.queryByText("1 이상 60 이하의 정수를 입력하세요.")).not.toBeInTheDocument()
  })

  it("T-ui-set-ss-02 [External] a rejected save shows the backend error inline and keeps the saved value", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    vi.spyOn(api, "updateSettings").mockRejectedValue({ code: "invalid_input", message: "bad" })
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openSettingsPage(user, "플레이어")
    const input = await within(dialog).findByRole("spinbutton", { name: "연속 재생 이미지 표시 시간" })
    // Act
    fireEvent.change(input, { target: { value: "9" } })
    fireEvent.keyDown(input, { key: "Enter" })
    // Assert
    expect(await within(dialog).findByRole("alert")).toBeInTheDocument()
    expect((await api.getSettings()).slideshowImageSeconds).toBe(5)
  })
})
