// Categories: Happy (T-ui-scene-08a section + toggles, 08b backfill confirm) / Boundary (08c zero count) / External (08d backfill failure)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

vi.mock("sonner", () => ({ Toaster: () => null, toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.clearAllMocks()
})

async function setup(count = 1234) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const api = createMockApi({ count: 50 })
  let remaining = count
  vi.spyOn(api, "sceneBackfillCount").mockImplementation(async () => remaining)
  const backfill = vi.spyOn(api, "sceneDetectBackfill").mockImplementation(async () => {
    const queued = remaining
    remaining = 0
    return { queued }
  })
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, "플레이어")
  const section = await within(page).findByRole("region", { name: "장면전환점" })
  return { api, backfill, update, user, section }
}

describe("settings > player > scene cuts", () => {
  it("T-ui-scene-08a [Happy] defaults (visible on, auto off with the CPU note, 보통); switches and the sensitivity select save their settings keys", async () => {
    // Arrange
    const { update, user, section } = await setup()
    const visible = within(section).getByRole("switch", { name: "탐색 바에 장면전환점 표시" })
    const auto = within(section).getByRole("switch", { name: "새 동영상 장면전환점 자동 추출" })
    // Assert: 기본값
    expect(visible).toBeChecked()
    expect(auto).not.toBeChecked()
    expect(within(section).getByText(/CPU 를 많이 씁니다/)).toBeInTheDocument()
    expect(within(section).getByRole("combobox", { name: "검출 민감도" })).toHaveTextContent("보통")
    // Act + Assert
    await user.click(auto)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ sceneDetectAuto: true }))
    await waitFor(() => expect(auto).toBeChecked())
    await user.click(visible)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ sceneCutsVisible: false }))
    await user.click(within(section).getByRole("combobox", { name: "검출 민감도" }))
    await user.click(await screen.findByRole("option", { name: "높음" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ sceneDetectSensitivity: "high" }))
  })

  it("T-ui-scene-08b [Happy] 모두 추출 (영상 n개) asks in an AlertDialog with the count; cancel does nothing, confirm enqueues once, toasts the queued count and refreshes the count", async () => {
    // Arrange
    const { backfill, user, section } = await setup(1234)
    const button = within(section).getByRole("button", { name: "모두 추출 (영상 1,234개)" })
    // Act 1: 취소
    await user.click(button)
    const dialog = await screen.findByRole("alertdialog")
    expect(dialog).toHaveTextContent("동영상 1,234개를 백그라운드 대기열에 등록합니다")
    await user.click(within(dialog).getByRole("button", { name: "취소" }))
    expect(backfill).not.toHaveBeenCalled()
    // Act 2: 확인
    await user.click(button)
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "추출 시작" }))
    // Assert: 대기열 등록만 하고 끝난다 (추출 완료를 기다리지 않는다)
    await waitFor(() => expect(backfill).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("동영상 1,234개의 장면전환점 추출을 대기열에 등록했습니다."))
    await waitFor(() => expect(within(section).getByRole("button", { name: "모두 추출 (영상 0개)" })).toBeDisabled())
  })

  it("T-ui-scene-08c [Boundary] with 0 targets the button is disabled and no dialog opens", async () => {
    const { user, section } = await setup(0)
    const button = within(section).getByRole("button", { name: "모두 추출 (영상 0개)" })
    expect(button).toBeDisabled()
    await user.click(button)
    expect(screen.queryByRole("alertdialog")).toBeNull()
  })

  it("T-ui-scene-08d [External] a failed backfill shows an error toast and no success toast", async () => {
    // Arrange
    const { backfill, user, section } = await setup(5)
    backfill.mockRejectedValueOnce({ code: "internal", message: "boom" })
    // Act
    await user.click(within(section).getByRole("button", { name: "모두 추출 (영상 5개)" }))
    await user.click(within(await screen.findByRole("alertdialog")).getByRole("button", { name: "추출 시작" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    expect(toast.success).not.toHaveBeenCalled()
  })
})
