// Categories: Business rule (T-ui-protect-01, -03) / Validation (T-ui-protect-02) / Happy (T-ui-protect-04)
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

type User = ReturnType<typeof userEvent.setup>

async function selectTwo(user: User) {
  const options = await screen.findAllByRole("option")
  await user.click(options[0])
  await user.keyboard("{Control>}")
  await user.click(options[1])
  await user.keyboard("{/Control}")
  return screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
}

/** 일괄 바 삭제 → 확인 AlertDialog 의 "휴지통으로 이동" 까지 진행한다. */
async function confirmDelete(user: User) {
  const bar = await selectTwo(user)
  await user.click(await within(bar).findByRole("button", { name: "삭제" }))
  const alert = await screen.findByRole("alertdialog", { name: "원본 파일 2개를 삭제할까요?" })
  expect(within(alert).getByText(/Windows 휴지통/)).toBeInTheDocument()
  await user.click(within(alert).getByRole("button", { name: "휴지통으로 이동" }))
}

describe("protection flow (P2-9)", () => {
  it("T-ui-protect-01 [Business rule] delete originals: confirm -> password -> command, then skipped within exemption", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, password: "correct-horse" })
    await api.updateSettings({ originalActionsEnabled: true }) // 이동·삭제는 설정이 켜져 있을 때만 노출
    const deleteSpy = vi.spyOn(api, "deleteOriginals")
    const user = userEvent.setup()
    renderApp(api)
    // Act 1: 확인 → 비밀번호 Dialog
    await confirmDelete(user)
    const prompt = await screen.findByRole("dialog", { name: "비밀번호 확인" })
    expect(deleteSpy).not.toHaveBeenCalled() // 비밀번호 전에는 command 를 부르지 않는다
    await user.type(within(prompt).getByLabelText("비밀번호"), "correct-horse")
    await user.click(within(prompt).getByRole("button", { name: "확인" }))
    // Assert 1
    await waitFor(() => expect(deleteSpy).toHaveBeenCalledTimes(1))
    expect(deleteSpy.mock.calls[0][0]).toHaveLength(2)
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "비밀번호 확인" })).not.toBeInTheDocument())
    // Act 2: 면제 시간 안 — 다시 삭제하면 비밀번호를 묻지 않는다
    await confirmDelete(user)
    // Assert 2
    await waitFor(() => expect(deleteSpy).toHaveBeenCalledTimes(2))
    expect(screen.queryByRole("dialog", { name: "비밀번호 확인" })).not.toBeInTheDocument()
  })

  it("T-ui-protect-02 [Validation] wrong password shows inline error with aria-invalid and never calls the command", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, password: "correct-horse" })
    await api.updateSettings({ originalActionsEnabled: true }) // 이동·삭제는 설정이 켜져 있을 때만 노출
    const deleteSpy = vi.spyOn(api, "deleteOriginals")
    const user = userEvent.setup()
    renderApp(api)
    await confirmDelete(user)
    const prompt = await screen.findByRole("dialog", { name: "비밀번호 확인" })
    // Act
    await user.type(within(prompt).getByLabelText("비밀번호"), "wrong-password")
    await user.click(within(prompt).getByRole("button", { name: "확인" }))
    // Assert
    const alert = await within(prompt).findByRole("alert")
    expect(alert).toHaveTextContent("비밀번호가 올바르지 않습니다.")
    const input = within(prompt).getByLabelText("비밀번호")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(input).toHaveAttribute("aria-describedby", alert.id)
    expect(deleteSpy).not.toHaveBeenCalled()
    // 취소해도 command 는 호출되지 않는다
    await user.click(within(prompt).getByRole("button", { name: "취소" }))
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "비밀번호 확인" })).not.toBeInTheDocument())
    expect(deleteSpy).not.toHaveBeenCalled()
  })

  it("T-ui-protect-02b [Business rule] no password set -> guide dialog, command not called", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    await api.updateSettings({ originalActionsEnabled: true }) // 이동·삭제는 설정이 켜져 있을 때만 노출
    const deleteSpy = vi.spyOn(api, "deleteOriginals")
    const user = userEvent.setup()
    renderApp(api)
    // Act
    await confirmDelete(user)
    // Assert
    const guide = await screen.findByRole("alertdialog", { name: "보호 비밀번호가 필요합니다" })
    expect(deleteSpy).not.toHaveBeenCalled()
    await user.click(within(guide).getByRole("button", { name: "보호 설정 열기" }))
    const nav = await screen.findByRole("navigation", { name: "설정 메뉴" })
    expect(within(nav).getByRole("button", { name: "보호" })).toHaveAttribute("aria-current", "page")
  })

  it("T-ui-protect-03 [Business rule] reset via forgot-password path shows originals-preserved text and restart notice", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, password: "correct-horse" })
    const resetSpy = vi.spyOn(api, "resetManagedData")
    const user = userEvent.setup()
    renderApp(api)
    const settings = await openSettingsPage(user, "데이터·백업")
    // Act
    await user.click(await within(settings).findByRole("button", { name: "관리 데이터 초기화" }))
    const warn = await screen.findByRole("alertdialog", { name: "관리 데이터를 초기화할까요?" })
    expect(within(warn).getByText("원본 미디어 파일은 삭제하거나 변경하지 않습니다.")).toBeInTheDocument()
    expect(within(warn).getByText(/자동으로 백업을 1회/)).toBeInTheDocument()
    await user.click(await within(warn).findByRole("checkbox", { name: "비밀번호를 잊었습니다" }))
    expect(within(warn).getByLabelText("비밀번호")).toBeDisabled()
    await user.click(within(warn).getByRole("button", { name: "초기화" }))
    // Assert
    await waitFor(() => expect(resetSpy).toHaveBeenCalledWith({ password: null, forgotPassword: true }))
    const done = await screen.findByRole("alertdialog", { name: "초기화를 마쳤습니다" })
    expect(done).toHaveTextContent("다시 시작")
    expect(within(done).queryByRole("button")).not.toBeInTheDocument() // 차단 안내 — 닫을 수단 없음
  })

  it("T-ui-protect-04 [Happy] onboarding: add folder -> finish saves firstRunCompleted", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300, firstRun: true })
    vi.spyOn(api, "pickDirectory").mockResolvedValue("E:\\Media")
    const updateSpy = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    // Act
    const dialog = await screen.findByRole("dialog", { name: "관리할 폴더를 추가하세요" })
    await user.click(within(dialog).getByRole("button", { name: "폴더 추가" }))
    const addDialog = await screen.findByRole("dialog", { name: "관리 폴더 추가" })
    await user.click(within(addDialog).getByRole("button", { name: "추가" }))
    expect(await within(dialog).findByText("E:\\Media")).toBeInTheDocument()
    await user.click(within(dialog).getByRole("button", { name: "다음" }))
    await user.click(await within(dialog).findByRole("button", { name: "나중에 설정" }))
    // Assert
    await waitFor(() => expect(updateSpy).toHaveBeenCalledWith({ firstRunCompleted: true }))
    expect((await api.getSettings()).firstRunCompleted).toBe(true)
    expect((await api.listFolders()).some((f) => f.path === "E:\\Media")).toBe(true)
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "관리할 폴더를 추가하세요" })).not.toBeInTheDocument())
  })
})
