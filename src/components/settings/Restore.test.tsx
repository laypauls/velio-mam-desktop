// Categories: Happy (T-ui-restore-01, -07) / Validation (T-ui-restore-03, -04) / Business rule (T-ui-restore-02, -05, -06, -08)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { UiApi } from "@/lib/api"
import { formatDateTime } from "@/lib/format"
import type { BackupInfo, StartupNotice } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const AT = Date.UTC(2026, 10, 6, 0, 0)
const BACKUPS: BackupInfo[] = [
  { fileName: "auto-2.db", createdAt: AT, sizeBytes: 12_000_000, kind: "auto" },
  { fileName: "pre-restore-1.db", createdAt: AT - 86_400_000, sizeBytes: 12_000_000, kind: "pre_restore" },
]
const DATE = formatDateTime(AT)

type User = ReturnType<typeof userEvent.setup>

async function openRestore(user: User, api: UiApi) {
  renderApp(api)
  const settings = await openSettingsPage(user, "데이터·백업")
  await user.click(await within(settings).findByRole("button", { name: `${DATE} 백업 복원` }))
  return screen.findByRole("alertdialog", { name: "이 백업으로 복원할까요?" })
}

describe("backup restore dialog (PR2)", () => {
  it("T-ui-restore-01 [Happy] row shows kind label, dialog states the backup date, no password field when unprotected, calls restoreBackup with fileName", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, backups: BACKUPS })
    const spy = vi.spyOn(api, "restoreBackup")
    const user = userEvent.setup()
    // Act
    const dialog = await openRestore(user, api)
    // Assert
    expect(within(dialog).getByText(new RegExp(`${DATE} 백업 시점`))).toBeInTheDocument()
    expect(screen.getByText("복원 전")).toBeInTheDocument()
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "복원하고 다시 시작" })).toBeEnabled())
    expect(within(dialog).queryByLabelText("비밀번호")).not.toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "복원하고 다시 시작" }))
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ fileName: "auto-2.db" }))
  })

  it("T-ui-restore-02 [Business rule] password set: field shown, empty submit blocked, correct password sends it", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, backups: BACKUPS, password: "correct-horse" })
    const spy = vi.spyOn(api, "restoreBackup")
    const user = userEvent.setup()
    const dialog = await openRestore(user, api)
    const field = await within(dialog).findByLabelText("비밀번호")
    // Act 1: 빈 입력
    await user.click(within(dialog).getByRole("button", { name: "복원하고 다시 시작" }))
    // Assert 1
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("비밀번호를 입력하세요.")
    expect(spy).not.toHaveBeenCalled()
    // Act 2
    await user.type(field, "correct-horse")
    await user.click(within(dialog).getByRole("button", { name: "복원하고 다시 시작" }))
    // Assert 2
    await waitFor(() => expect(spy).toHaveBeenCalledWith({ fileName: "auto-2.db", password: "correct-horse" }))
  })

  it("T-ui-restore-03 [Validation] wrong password shows inline error (aria-invalid/describedby), dialog stays open and can retry", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, backups: BACKUPS, password: "correct-horse" })
    const user = userEvent.setup()
    const dialog = await openRestore(user, api)
    // Act
    await user.type(await within(dialog).findByLabelText("비밀번호"), "wrong-password")
    await user.click(within(dialog).getByRole("button", { name: "복원하고 다시 시작" }))
    // Assert
    const alert = await within(dialog).findByRole("alert")
    expect(alert).toHaveTextContent("비밀번호가 올바르지 않습니다.")
    const input = within(dialog).getByLabelText("비밀번호")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input).toHaveAttribute("aria-describedby", alert.id)
    expect(within(dialog).getByRole("button", { name: "복원하고 다시 시작" })).toBeEnabled()
  })

  it("T-ui-restore-04 [Validation] backup_invalid / backup_too_new errors map to inline messages when no password is set", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, backups: BACKUPS })
    vi.spyOn(api, "restoreBackup").mockRejectedValueOnce({ code: "backup_too_new", message: "m" })
    const user = userEvent.setup()
    const dialog = await openRestore(user, api)
    await waitFor(() => expect(within(dialog).getByRole("button", { name: "복원하고 다시 시작" })).toBeEnabled())
    // Act
    await user.click(within(dialog).getByRole("button", { name: "복원하고 다시 시작" }))
    // Assert
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(/새로운 버전에서 만든 백업/)
  })

  it("T-ui-restore-05 [Business rule] cancel closes without calling restoreBackup", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, backups: BACKUPS })
    const spy = vi.spyOn(api, "restoreBackup")
    const user = userEvent.setup()
    const dialog = await openRestore(user, api)
    // Act
    await user.click(within(dialog).getByRole("button", { name: "취소" }))
    // Assert
    await waitFor(() => expect(screen.queryByRole("alertdialog", { name: "이 백업으로 복원할까요?" })).not.toBeInTheDocument())
    expect(spy).not.toHaveBeenCalled()
  })
})

describe("startup notice (PR3)", () => {
  const recovered = (over: Partial<Extract<StartupNotice, { kind: "recovered" }>> = {}): StartupNotice => ({
    kind: "recovered",
    backupFile: "auto-2.db",
    createdAt: AT,
    corruptFile: "C:\\MAMData\\database\\mam.db.corrupt-1762",
    consentReset: true,
    ...over,
  })

  it("T-ui-restore-06 [Business rule] recovered with backup: dialog shows date, corrupt file path and consent-reset line", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, startupNotice: recovered() })
    const user = userEvent.setup()
    // Act
    renderApp(api)
    // Assert
    const dialog = await screen.findByRole("dialog", { name: "관리 데이터가 손상되어 복구했습니다" })
    expect(within(dialog).getByText(new RegExp(`${DATE} 백업으로 복원했습니다`))).toBeInTheDocument()
    expect(within(dialog).getByText("C:\\MAMData\\database\\mam.db.corrupt-1762")).toBeInTheDocument()
    expect(within(dialog).getByText("외부 AI 전송 동의가 해제되어 다시 동의해야 합니다.")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "확인" }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리 데이터가 손상되어 복구했습니다" })).not.toBeInTheDocument())
  })

  it("T-ui-restore-07 [Boundary] recovered without backup: empty-start message, no consent line when not reset", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({
      count: 300,
      startupNotice: recovered({ backupFile: null, createdAt: null, consentReset: false }),
    })
    // Act
    renderApp(api)
    // Assert
    const dialog = await screen.findByRole("dialog", { name: "관리 데이터가 손상되어 복구했습니다" })
    expect(within(dialog).getByText(/쓸 수 있는 백업이 없어 빈 데이터로 시작했습니다/)).toBeInTheDocument()
    expect(within(dialog).queryByText(/동의가 해제/)).not.toBeInTheDocument()
  })

  it("T-ui-restore-08 [Business rule] restored shows a toast (no dialog); notice is fetched exactly once and not again after re-render", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, startupNotice: { kind: "restored", backupFile: "auto-2.db", createdAt: AT } })
    const spy = vi.spyOn(api, "takeStartupNotice")
    const user = userEvent.setup()
    // Act
    renderApp(api)
    // Assert
    expect(await screen.findByText(new RegExp(`백업\\(${DATE}\\)으로 복원했습니다`))).toBeInTheDocument()
    expect(screen.queryByRole("dialog", { name: "관리 데이터가 손상되어 복구했습니다" })).not.toBeInTheDocument()
    // 설정을 열고 닫아 리렌더를 유발해도 다시 읽지 않는다
    await openSettingsPage(user)
    expect(spy).toHaveBeenCalledTimes(1)
  })

  it("T-ui-restore-09 [Boundary] no notice: nothing shown; mock notice is read-once", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, startupNotice: { kind: "restored", backupFile: "a.db", createdAt: AT } })
    // Act
    const first = await api.takeStartupNotice()
    const second = await api.takeStartupNotice()
    // Assert
    expect(first?.kind).toBe("restored")
    expect(second).toBeNull()
    renderApp(createMockApi({ count: 300 }))
    await screen.findByRole("button", { name: "설정" })
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument()
  })
})
