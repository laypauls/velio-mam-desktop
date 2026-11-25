// Categories: Happy (T-ui-vlcset-12, T-ui-hidekey-20/22) / Business rule (T-ui-hidekey-21) / Validation (T-ui-hidekey-23) / Boundary (T-ui-hidekey-24) / Business rule (T-ui-player-01 page split)
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

async function openPlayer(user: ReturnType<typeof userEvent.setup>) {
  const dialog = await openSettingsPage(user, "플레이어")
  await within(dialog).findByRole("region", { name: "플레이어" })
  return dialog
}

describe("settings > player > page split", () => {
  it("T-ui-player-01 [Business rule] player rows live on the 플레이어 page only; 탐색·표시 keeps double-click and sort fields", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    // Act
    const player = await openPlayer(user)
    // Assert: 플레이어 페이지
    for (const name of ["동영상을 내장 플레이어로 재생"]) expect(within(player).getByRole("switch", { name })).toBeInTheDocument()
    expect(within(player).getByRole("radiogroup", { name: "동영상·음성 열 때" })).toBeInTheDocument()
    expect(within(player).getByRole("spinbutton", { name: "연속 재생 이미지 표시 시간" })).toBeInTheDocument()
    expect(within(player).getByRole("button", { name: "재생 정지 후 숨기기 단축키" })).toBeInTheDocument()
    expect(within(player).queryByRole("group", { name: "정렬 항목" })).not.toBeInTheDocument()
    // Act: 탐색·표시
    const nav = within(await screen.findByRole("navigation", { name: "설정 메뉴" }))
    await user.click(nav.getByRole("button", { name: "탐색·표시" }))
    // Assert
    const view = await within(screen.getByRole("main")).findByRole("group", { name: "정렬 항목" })
    expect(view).toBeInTheDocument()
    expect(screen.queryByRole("switch", { name: "동영상을 내장 플레이어로 재생" })).not.toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "재생 정지 후 숨기기 단축키" })).not.toBeInTheDocument()
    expect(screen.getByRole("combobox", { name: "더블클릭 동작" })).toBeInTheDocument()
  })
})

describe("settings > player > video player", () => {
  it("T-ui-vlcset-12 [Happy] the built-in player switch defaults ON and saves { videoPlayerVlc } OFF then ON", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openPlayer(user)
    const sw = within(dialog).getByRole("switch", { name: "동영상을 내장 플레이어로 재생" })
    expect(sw).toBeChecked()
    // Act + Assert: OFF
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ videoPlayerVlc: false }))
    await waitFor(() => expect(sw).not.toBeChecked())
    // Act + Assert: ON
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ videoPlayerVlc: true }))
    expect((await api.getSettings()).videoPlayerVlc).toBe(true)
  })
})

describe("settings > player > hide shortcut", () => {
  async function setup() {
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const hide = vi.spyOn(api, "hideMainWindow").mockResolvedValue(undefined)
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openPlayer(user)
    const field = within(dialog).getByRole("button", { name: "재생 정지 후 숨기기 단축키" })
    return { api, update, hide, user, dialog, field }
  }

  it("T-ui-hidekey-20 [Happy] shows Esc by default; click -> prompt -> next key is recorded and saved", async () => {
    // Arrange
    const { update, user, field, api } = await setup()
    expect(field).toHaveTextContent("Esc")
    // Act
    await user.click(field)
    expect(field).toHaveTextContent("키를 누르세요…")
    await user.keyboard("{Control>}{Shift>}h{/Shift}{/Control}")
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ hideShortcut: "Ctrl+Shift+H" }))
    await waitFor(() => expect(field).toHaveTextContent("Ctrl+Shift+H"))
    expect((await api.getSettings()).hideShortcut).toBe("Ctrl+Shift+H")
  })

  it("T-ui-hidekey-21 [Business rule] Esc can be recorded (it does not hide the window meanwhile); 기본값 restores Escape", async () => {
    // Arrange
    const { update, hide, user, dialog, field } = await setup()
    await user.click(field)
    await user.keyboard("{F9}")
    await waitFor(() => expect(field).toHaveTextContent("F9"))
    // Act: Esc 기록
    await user.click(field)
    await user.keyboard("{Escape}")
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ hideShortcut: "Escape" }))
    expect(hide).not.toHaveBeenCalled()
    // Act: 다른 키로 바꾼 뒤 기본값
    await user.click(field)
    await user.keyboard("{F9}")
    await waitFor(() => expect(field).toHaveTextContent("F9"))
    await user.click(within(dialog).getAllByRole("button", { name: "기본값" })[1]) // 숨기기 행은 패널 닫기 행 아래
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ hideShortcut: "Escape" }))
    await waitFor(() => expect(field).toHaveTextContent("Esc"))
  })

  it("T-ui-hidekey-22 [Happy] 사용 안 함 saves null and shows the label", async () => {
    // Arrange
    const { update, user, dialog, field } = await setup()
    // Act
    await user.click(within(dialog).getAllByRole("button", { name: "사용 안 함" })[1]) // 숨기기 행은 패널 닫기 행 아래
    // Assert
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ hideShortcut: null }))
    await waitFor(() => expect(field).toHaveTextContent("사용 안 함"))
  })

  it("T-ui-hidekey-23 [Validation] a key used by the app is rejected with an inline error (aria-invalid + describedby) and nothing is saved", async () => {
    // Arrange
    const { update, user, field } = await setup()
    // Act
    await user.click(field)
    await user.keyboard(" ")
    // Assert
    const alert = await screen.findByRole("alert")
    expect(alert).toHaveTextContent("Space 키는 다른 기능에 쓰이고 있습니다")
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(field.getAttribute("aria-describedby")).toContain(alert.id)
    expect(update).not.toHaveBeenCalled()
    expect(field).toHaveTextContent("Esc")
    // 다음 시도(유효한 키)에서 에러가 사라진다
    await user.click(field)
    await user.keyboard("{F8}")
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ hideShortcut: "F8" }))
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
  })

  it("T-ui-hidekey-24 [Boundary] Tab or blur cancels recording; a lone modifier keeps waiting", async () => {
    // Arrange
    const { update, user, field } = await setup()
    // Act: 수정자만 → 계속 대기
    await user.click(field)
    await user.keyboard("{Shift}")
    expect(field).toHaveTextContent("키를 누르세요…")
    // Act: Tab → 취소
    await user.keyboard("{Tab}")
    // Assert
    expect(field).toHaveTextContent("Esc")
    expect(update).not.toHaveBeenCalled()
  })
})
