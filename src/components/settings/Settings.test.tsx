// Categories: Validation + Happy (T-ui-settings-01) / Business rule + External (T-ui-settings-02~04)
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

async function openTab(user: User, tab: string) {
  return openSettingsPage(user, tab)
}

describe("settings page (P2-9)", () => {
  it("T-ui-settings-01 [Validation/Happy] backup keep validates inline, saves valid value, backup now lists and shows disk_full inline", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const updateSpy = vi.spyOn(api, "updateSettings")
    const backupSpy = vi.spyOn(api, "backupNow")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openTab(user, "데이터·백업")
    const keep = await within(dialog).findByLabelText("보관 개수")
    // Act 1: 범위 밖 값
    await user.clear(keep)
    await user.type(keep, "0")
    await user.tab()
    // Assert 1
    const error = await within(dialog).findByRole("alert")
    expect(error).toHaveTextContent("1 이상 365 이하의 정수를 입력하세요.")
    expect(keep).toHaveAttribute("aria-invalid", "true")
    expect(keep).toHaveAttribute("aria-describedby", error.id)
    expect(updateSpy).not.toHaveBeenCalled()
    // Act 2: 올바른 값
    await user.clear(keep)
    await user.type(keep, "3")
    await user.tab()
    // Assert 2
    await waitFor(() => expect(updateSpy).toHaveBeenCalledWith({ backup: { keep: 3 } }))
    expect((await api.getSettings()).backup.keep).toBe(3)
    expect(keep).not.toHaveAttribute("aria-invalid")
    // Act 3: 지금 백업
    await user.click(within(dialog).getByRole("button", { name: "지금 백업" }))
    // Assert 3
    await waitFor(() => expect(backupSpy).toHaveBeenCalledTimes(1))
    const list = await within(dialog).findByRole("list", { name: "백업 목록" })
    expect(within(list).getByText("수동")).toBeInTheDocument()
    // Act 4: 디스크 부족
    backupSpy.mockRejectedValueOnce({ code: "disk_full", message: "disk full" })
    await user.click(within(dialog).getByRole("button", { name: "지금 백업" }))
    // Assert 4
    expect(await within(dialog).findByText("저장 공간이 부족합니다.")).toBeInTheDocument()
  })

  it("T-ui-settings-02 [Business rule] adding an already registered / nested folder shows an inline error", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const existing = (await api.listFolders())[0].path
    const pick = vi.spyOn(api, "pickDirectory").mockResolvedValue(existing)
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openTab(user, "관리 폴더")
    await within(dialog).findByRole("table", { name: "관리 폴더" })
    const before = (await api.listFolders()).length
    // Act 1: 이미 등록된 경로
    await user.click(within(dialog).getByRole("button", { name: "폴더 추가" }))
    const addDialog = await screen.findByRole("dialog", { name: "관리 폴더 추가" })
    await user.click(within(addDialog).getByRole("button", { name: "추가" }))
    // Assert 1: 유형 선택 Dialog 안에 인라인으로 표시된다
    expect(await within(addDialog).findByRole("alert")).toHaveTextContent("이미 등록되어 있습니다.")
    expect((await api.listFolders()).length).toBe(before)
    await user.click(within(addDialog).getByRole("button", { name: "취소" }))
    // Act 2: 겹치는 경로
    pick.mockResolvedValue(`${existing}\\sub`)
    await user.click(within(dialog).getByRole("button", { name: "폴더 추가" }))
    const addDialog2 = await screen.findByRole("dialog", { name: "관리 폴더 추가" })
    await user.click(within(addDialog2).getByRole("button", { name: "추가" }))
    // Assert 2
    await waitFor(() => expect(within(addDialog2).getByRole("alert")).toHaveTextContent("관리 폴더 또는 MAM 데이터·백업 폴더와 겹치는 위치입니다."))
    expect((await api.listFolders()).length).toBe(before)
  })

  it("T-ui-settings-03 [Business rule] data dir rejects network path inline and does not save", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    vi.spyOn(api, "pickDirectory").mockResolvedValue("\\\\NAS\\share\\MAMData")
    const updateSpy = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openTab(user, "데이터·백업")
    // Act
    const location = await within(dialog).findByRole("region", { name: "관리 데이터 위치" })
    await user.click(within(location).getByRole("button", { name: "관리 데이터 위치 변경" }))
    // Assert
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("네트워크 경로는 사용할 수 없습니다")
    expect(updateSpy).not.toHaveBeenCalled()
  })

  it("T-ui-settings-04 [Business rule] raising the auth exemption asks for the password, lowering saves directly", async () => {
    // Arrange: 면제 5분(기본), 비밀번호 설정됨, 면제 중 아님
    stubLayout()
    const api = createMockApi({ count: 300, password: "correct-horse" })
    const updateSpy = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openTab(user, "보호")
    const field = await within(dialog).findByLabelText("인증 면제 시간")
    // Act 1: 올리기(5 → 10) → 비밀번호 확인 후 저장
    await user.clear(field)
    await user.type(field, "10")
    await user.tab()
    const prompt = await screen.findByRole("dialog", { name: "비밀번호 확인" })
    expect(updateSpy).not.toHaveBeenCalled()
    await user.type(within(prompt).getByLabelText("비밀번호"), "correct-horse")
    await user.click(within(prompt).getByRole("button", { name: "확인" }))
    // Assert 1
    await waitFor(() => expect(updateSpy).toHaveBeenCalledWith({ authExemptMinutes: 10 }))
    expect((await api.getSettings()).authExemptMinutes).toBe(10)
    // Act 2: 내리기(10 → 3) → 비밀번호 없이 바로 저장
    updateSpy.mockClear()
    const again = await within(dialog).findByLabelText("인증 면제 시간")
    await user.clear(again)
    await user.type(again, "3")
    await user.tab()
    // Assert 2
    await waitFor(() => expect(updateSpy).toHaveBeenCalledWith({ authExemptMinutes: 3 }))
    expect(screen.queryByRole("dialog", { name: "비밀번호 확인" })).not.toBeInTheDocument()
  })
})
