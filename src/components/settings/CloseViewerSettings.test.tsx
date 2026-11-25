// Categories: Happy (default, record, default button, disable, IME) / Validation (same combo both ways, reserved, range preset) / Business rule (section warning)
import { fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { Settings } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(patch: Partial<Settings> = {}) {
  stubLayout()
  const api = createMockApi({ count: 50 })
  await api.updateSettings(patch)
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, "플레이어")
  await within(page).findByRole("region", { name: "플레이어" })
  const close = within(page).getByRole("button", { name: "플레이어 패널 닫기" })
  const hide = within(page).getByRole("button", { name: "재생 정지 후 숨기기 단축키" })
  return { api, update, user, page, close, hide }
}

async function expectRejected(field: HTMLElement, update: ReturnType<typeof vi.spyOn>, text: string) {
  const alert = await screen.findByRole("alert")
  expect(alert).toHaveTextContent(text)
  expect(field).toHaveAttribute("aria-invalid", "true")
  expect(field.getAttribute("aria-describedby")).toContain(alert.id)
  expect(update).not.toHaveBeenCalled()
}

describe("settings > player > close viewer shortcut", () => {
  it("T-ui-closekey-20 [Happy] sits right above the hide row, shows Q by default with its hint; a recorded key is saved", async () => {
    // Arrange
    const { update, user, close, hide, api } = await setup()
    // Assert: 기본값·순서·안내
    expect(close).toHaveTextContent("Q")
    expect(screen.getByText("재생을 멈추고 플레이어 패널만 닫습니다. 앱은 그대로 둡니다.")).toBeInTheDocument()
    expect(close.compareDocumentPosition(hide) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    // Act
    await user.click(close)
    await user.keyboard("{F7}")
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ closeViewerShortcut: "F7" }))
    await waitFor(() => expect(close).toHaveTextContent("F7"))
    expect((await api.getSettings()).closeViewerShortcut).toBe("F7")
  })

  it("T-ui-closekey-21 [Happy] Esc is allowed; 기본값 restores Q; 사용 안 함 saves null", async () => {
    // Arrange
    const { update, user, close, page } = await setup({ hideShortcut: "F9" })
    // Act: Esc
    await user.click(close)
    await user.keyboard("{Escape}")
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ closeViewerShortcut: "Escape" }))
    // Act: 기본값 (행 안의 버튼 — 두 행에 같은 이름이 있어 첫 번째가 닫기 행)
    await user.click(within(page).getAllByRole("button", { name: "기본값" })[0])
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ closeViewerShortcut: "Q" }))
    await waitFor(() => expect(close).toHaveTextContent("Q"))
    // Act: 사용 안 함
    await user.click(within(page).getAllByRole("button", { name: "사용 안 함" })[0])
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ closeViewerShortcut: null }))
    await waitFor(() => expect(close).toHaveTextContent("사용 안 함"))
  })

  it("T-ui-closekey-22 [Validation] the hide shortcut's combo is rejected in the close row", async () => {
    // Arrange
    const { update, user, close } = await setup({ hideShortcut: "F9" })
    // Act
    await user.click(close)
    await user.keyboard("{F9}")
    // Assert
    await expectRejected(close, update, "F9 키는 다른 기능에 쓰이고 있습니다")
    expect(close).toHaveTextContent("Q")
  })

  it("T-ui-closekey-23 [Validation] the close shortcut's combo is rejected in the hide row (both directions)", async () => {
    // Arrange
    const { update, user, hide } = await setup()
    // Act
    await user.click(hide)
    await user.keyboard("q")
    // Assert
    await expectRejected(hide, update, "Q 키는 다른 기능에 쓰이고 있습니다")
    expect(hide).toHaveTextContent("Esc")
  })

  it("T-ui-closekey-24 [Validation] reserved keys are rejected", async () => {
    // Arrange
    const { update, user, close } = await setup()
    // Act
    await user.click(close)
    await user.keyboard("m")
    // Assert
    await expectRejected(close, update, "M 키는 다른 기능에 쓰이고 있습니다")
  })

  it("T-ui-closekey-25 [Validation] a key of the selected range preset is rejected (I with Premiere)", async () => {
    // Arrange
    const { update, user, close } = await setup({ playRangeKeys: "premiere" })
    // Act
    await user.click(close)
    await user.keyboard("i")
    // Assert
    await expectRejected(close, update, "I 키는 다른 기능에 쓰이고 있습니다")
  })

  it("T-ui-closekey-26 [Validation] with the brackets preset the same key I is saved", async () => {
    // Arrange
    const { update, user, close } = await setup({ playRangeKeys: "brackets" })
    // Act
    await user.click(close)
    await user.keyboard("i")
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ closeViewerShortcut: "I" }))
  })

  it("T-ui-closekey-27 [Happy] recording on the Korean IME (key ㅍ, code KeyV) stores the Latin letter V", async () => {
    // Arrange
    const { update, user, close } = await setup()
    await user.click(close)
    // Act
    fireEvent.keyDown(close, { key: "ㅍ", code: "KeyV" })
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ closeViewerShortcut: "V" }))
  })

  it("T-ui-closekey-28 [Business rule] a stored value that collides with the range preset shows one unified warning linked to the preset control", async () => {
    // Arrange + Act
    const { page } = await setup({ closeViewerShortcut: "I", hideShortcut: "O", playRangeKeys: "premiere" })
    // Assert
    const warnings = within(page).getAllByRole("status").filter((el) => el.textContent?.includes("겹칩니다"))
    expect(warnings).toHaveLength(1)
    expect(warnings[0]).toHaveTextContent("플레이어 패널 닫기 단축키와 I 키가 겹칩니다")
    expect(warnings[0]).toHaveTextContent("숨기기 단축키와 O 키가 겹칩니다")
    const select = within(page).getByRole("radiogroup", { name: "재생 구간 단축키" })
    expect(select).toHaveAttribute("aria-invalid", "true")
    expect(select).toHaveAttribute("aria-describedby", warnings[0].id)
  })
})
