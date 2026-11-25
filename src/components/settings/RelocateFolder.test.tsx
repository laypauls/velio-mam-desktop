// Categories: Happy (01, 02) / Boundary (01 null pick) / External (03)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openFolderSettings } from "@/test/folderDialog"
import { openSettingsPage } from "@/test/settings"

const OLD_PATH = "D:\\Photos"
const NEW_PATH = "E:\\Photos2"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(newPath: string | null = NEW_PATH) {
  stubLayout()
  const user = userEvent.setup()
  const api = createMockApi({ count: 10 })
  vi.spyOn(api, "pickDirectory").mockResolvedValue(newPath)
  renderApp(api)
  const settings = await openSettingsPage(user, "관리 폴더")
  const settingsDialog = await openFolderSettings(user, settings, OLD_PATH)
  const button = within(settingsDialog).getByRole("button", { name: "폴더 변경" })
  return { user, api, settings, settingsDialog, button }
}

describe("relocate managed folder UI", () => {
  it("T-ui-relocate-01 [Happy] button -> pick -> dialog shows current and new path", async () => {
    // Arrange
    const { user, button } = await setup()
    // Act
    await user.click(button)
    // Assert
    const dialog = await screen.findByRole("dialog", { name: "관리 폴더 경로 변경" })
    expect(within(dialog).getByText(OLD_PATH)).toBeInTheDocument()
    expect(within(dialog).getByText(NEW_PATH)).toBeInTheDocument()
    expect(within(dialog).getByText(/원본 파일은 이동하거나 수정하지 않습니다/)).toBeInTheDocument()
  })

  it("T-ui-relocate-01b [Boundary] pick cancelled -> no dialog; picker error -> no dialog", async () => {
    // Arrange
    const { user, api, button } = await setup(null)
    // Act
    await user.click(button)
    // Assert
    expect(api.pickDirectory).toHaveBeenCalledTimes(1)
    expect(screen.queryByRole("dialog", { name: "관리 폴더 경로 변경" })).not.toBeInTheDocument()
    // Act (error)
    vi.spyOn(api, "pickDirectory").mockRejectedValue({ code: "io", message: "x" })
    const errSpy = vi.spyOn(toast, "error")
    await user.click(button)
    // Assert
    await waitFor(() => expect(errSpy).toHaveBeenCalledWith("파일 입출력 중 오류가 발생했습니다."))
    expect(screen.queryByRole("dialog", { name: "관리 폴더 경로 변경" })).not.toBeInTheDocument()
  })

  it("T-ui-relocate-01c [Business rule] relocate button is enabled for an offline folder", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 10 }))
    const settings = await openSettingsPage(user, "관리 폴더")
    // Act: 오프라인 폴더(NAS)의 설정
    const dialog = await openFolderSettings(user, settings, "\\\\NAS\\Archive")
    // Assert: 오프라인 폴더는 재스캔이 비활성이지만 폴더 변경은 활성
    expect(within(dialog).getByRole("button", { name: "지금 재스캔" })).toBeDisabled()
    expect(within(dialog).getByRole("button", { name: "폴더 변경" })).toBeEnabled()
  })

  it("T-ui-relocate-02 [Happy] confirm calls relocateFolder, toasts, closes the dialog and refetches folders", async () => {
    // Arrange
    const { user, api, settings, button } = await setup()
    const relocate = vi.spyOn(api, "relocateFolder")
    const listSpy = vi.spyOn(api, "listFolders")
    const success = vi.spyOn(toast, "success")
    await user.click(button)
    const dialog = await screen.findByRole("dialog", { name: "관리 폴더 경로 변경" })
    // Act
    await user.click(within(dialog).getByRole("button", { name: "경로 변경" }))
    // Assert
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 폴더 경로 변경" })).not.toBeInTheDocument())
    expect(relocate).toHaveBeenCalledWith(1, NEW_PATH)
    expect(success).toHaveBeenCalledWith(expect.stringContaining(NEW_PATH))
    await waitFor(() => expect(listSpy).toHaveBeenCalled())
    expect(await within(settings).findByText("Photos2")).toBeInTheDocument()
    // 통합 다이얼로그는 열린 채 최신 경로를 보인다
    expect(within(screen.getByRole("dialog", { name: "관리 폴더 설정" })).getByText(NEW_PATH)).toBeInTheDocument()
  })

  it("T-ui-relocate-03 [External] scan_running shows inline alert, dialog stays; reopen clears it", async () => {
    // Arrange
    const { user, api, button } = await setup()
    vi.spyOn(api, "relocateFolder").mockRejectedValue({ code: "scan_running", message: "busy" })
    await user.click(button)
    const dialog = await screen.findByRole("dialog", { name: "관리 폴더 경로 변경" })
    // Act
    await user.click(within(dialog).getByRole("button", { name: "경로 변경" }))
    // Assert
    const alert = await within(dialog).findByRole("alert")
    expect(alert.textContent).toBe("이 폴더를 스캔하는 중입니다. 스캔을 일시정지한 뒤 다시 시도하세요.")
    expect(screen.getByRole("dialog", { name: "관리 폴더 경로 변경" })).toBeInTheDocument()
    // Act: 취소 후 다시 열기
    await user.click(within(dialog).getByRole("button", { name: "취소" }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 폴더 경로 변경" })).not.toBeInTheDocument())
    await user.click(button)
    // Assert
    const reopened = await screen.findByRole("dialog", { name: "관리 폴더 경로 변경" })
    expect(within(reopened).queryByRole("alert")).toBeNull()
  })
})
