// Categories: Happy (T-ui-sreorg-01~03, 04, 06) / Validation (T-ui-sreorg-04b) / Business rule (T-ui-sreorg-03, 05, 05b, 07) / Boundary (T-ui-sreorg-08 no conflict)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { SettingsPatch } from "@/lib/ipc-contract"
import { useProtectStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"
import { SETTINGS_MENU } from "./settingsMenu"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup(menu: string, settings: SettingsPatch = {}) {
  stubLayout()
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const api = createMockApi({ count: 50 })
  if (Object.keys(settings).length > 0) await api.updateSettings(settings)
  const update = vi.spyOn(api, "updateSettings")
  const user = userEvent.setup()
  renderApp(api)
  const page = await openSettingsPage(user, menu)
  return { api, update, user, page }
}

describe("settings reorg — menu and moved pages", () => {
  it("T-ui-sreorg-01 [Happy] the menu is a flat list of exactly 11 pages, without group headers or a 백업 page", async () => {
    // Arrange / Act
    const { page } = await setup("일반")
    const nav = within(await screen.findByRole("navigation", { name: "설정 메뉴" }))
    // Assert
    expect(nav.getAllByRole("button").map((b) => b.textContent)).toEqual([
      "일반",
      "탐색·표시",
      "플레이어",
      "이어보기",
      "관리 폴더",
      "메타데이터 필드",
      "외부 AI",
      "로컬 AI·검색",
      "보호",
      "데이터·백업",
      "플러그인",
      "오픈소스 라이선스",
    ])
    expect(SETTINGS_MENU).toHaveLength(12)
    expect(nav.queryByRole("heading")).not.toBeInTheDocument()
    expect(nav.queryByRole("button", { name: "백업" })).not.toBeInTheDocument()
    expect(page).toBeInTheDocument()
  })

  it("T-ui-sreorg-02 [Happy] openSettings('ai-local') opens the 로컬 AI·검색 page with semantic search and models, and without the consent section", async () => {
    // Arrange
    stubLayout()
    renderApp(createMockApi({ count: 50 }))
    // Act
    act(() => useProtectStore.getState().openSettings("ai-local"))
    // Assert
    const main = await screen.findByRole("main")
    expect(await within(main).findByRole("switch", { name: "의미 검색" })).toBeInTheDocument()
    expect(within(main).getByRole("region", { name: "유사 미디어" })).toBeInTheDocument()
    expect(within(main).queryByRole("region", { name: "외부 AI 전송 동의" })).not.toBeInTheDocument()
    expect(screen.getByRole("heading", { level: 2, name: "로컬 AI·검색" })).toBeInTheDocument()
  })

  it("T-ui-sreorg-03 [Business rule] the 외부 AI page keeps consent and no longer holds the local semantic-search switch", async () => {
    // Arrange / Act
    const { page } = await setup("외부 AI")
    // Assert
    expect(await within(page).findByRole("region", { name: "외부 AI 전송 동의" })).toBeInTheDocument()
    expect(within(page).queryByRole("switch", { name: "의미 검색" })).not.toBeInTheDocument()
    expect(within(page).queryByRole("region", { name: "유사 미디어" })).not.toBeInTheDocument()
  })

  it("T-ui-sreorg-04 [Happy] the rescan interval is edited and saved from 관리 폴더 > 스캔, and 일반 no longer has it", async () => {
    // Arrange
    const { api, update, user, page } = await setup("관리 폴더")
    const field = await within(page).findByLabelText("재스캔 간격")
    // Act
    await user.clear(field)
    await user.type(field, "12")
    await user.tab()
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ rescanIntervalHours: 12 }))
    expect((await api.getSettings()).rescanIntervalHours).toBe(12)
    expect(within(page).getByRole("region", { name: "스캔" })).toContainElement(field)
    // 일반 페이지에는 없다
    await user.click(within(screen.getByRole("navigation", { name: "설정 메뉴" })).getByRole("button", { name: "일반" }))
    await within(page).findByRole("switch", { name: "창을 닫으면 트레이로 보내기" })
    expect(within(page).queryByLabelText("재스캔 간격")).not.toBeInTheDocument()
  })

  it("T-ui-sreorg-04b [Validation] an out-of-range rescan interval shows the range error inline and does not save", async () => {
    // Arrange
    const { update, user, page } = await setup("관리 폴더")
    const field = await within(page).findByLabelText("재스캔 간격")
    // Act
    await user.clear(field)
    await user.type(field, "721")
    await user.tab()
    // Assert
    const error = await within(page).findByText("1 이상 720 이하의 정수를 입력하세요.")
    expect(field).toHaveAttribute("aria-invalid", "true")
    expect(field).toHaveAttribute("aria-describedby", error.id)
    expect(update).not.toHaveBeenCalled()
  })

  it("T-ui-sreorg-05 [Business rule] 데이터·백업 shows backup and data-location content together; reset sits inside the danger zone while location does not", async () => {
    // Arrange / Act
    const { page } = await setup("데이터·백업")
    await within(page).findByLabelText("보관 개수")
    // Assert: 백업 + 위치
    expect(within(page).getByRole("region", { name: "백업 목록" })).toBeInTheDocument()
    const location = within(page).getByRole("region", { name: "관리 데이터 위치" })
    const dataChange = within(location).getByRole("button", { name: "관리 데이터 위치 변경" })
    expect(dataChange).toHaveTextContent("위치 변경")
    // 같은 페이지의 백업 위치 버튼과 서로 다른 요소로 구분된다
    const backupChange = within(page).getByRole("button", { name: "백업 위치 변경" })
    expect(backupChange).toHaveTextContent("위치 변경")
    expect(backupChange).not.toBe(dataChange)
    // 초기화는 위험 영역 안에만 있다
    const danger = within(page).getByRole("region", { name: "위험 영역" })
    expect(within(danger).getByRole("button", { name: "관리 데이터 초기화" })).toBeInTheDocument()
    expect(within(location).queryByRole("button", { name: "관리 데이터 초기화" })).not.toBeInTheDocument()
    expect(within(page).getAllByRole("button", { name: "관리 데이터 초기화" })).toHaveLength(1)
  })

  it("T-ui-sreorg-05b [Business rule] 보호 puts 원본 이동·삭제 사용 inside the danger zone and keeps the exemption time outside", async () => {
    // Arrange / Act
    const { update, user, page } = await setup("보호")
    const danger = await within(page).findByRole("region", { name: "위험 영역" })
    // Assert
    const sw = within(danger).getByRole("switch", { name: "원본 파일 이동·삭제 사용" })
    expect(within(danger).queryByLabelText("인증 면제 시간")).not.toBeInTheDocument()
    expect(within(page).getByLabelText("인증 면제 시간")).toBeInTheDocument()
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenCalledWith({ originalActionsEnabled: true }))
  })
})

describe("settings reorg — player shortcuts subsection", () => {
  it("T-ui-sreorg-06 [Happy] 단축키 holds the range preset, the hide shortcut and the scene key guide; 장면전환점 no longer shows the guide", async () => {
    // Arrange / Act
    const { page } = await setup("플레이어")
    const shortcuts = await within(page).findByRole("region", { name: "단축키" })
    // Assert
    expect(within(shortcuts).getByRole("radiogroup", { name: "재생 구간 단축키" })).toBeInTheDocument()
    expect(within(shortcuts).getByRole("button", { name: "재생 정지 후 숨기기 단축키" })).toBeInTheDocument()
    expect(within(shortcuts).getByText(/이전\/다음 장면전환점 Ctrl\+←/)).toBeInTheDocument()
    const scene = within(page).getByRole("region", { name: "장면전환점" })
    expect(within(scene).queryByText(/이전\/다음 장면전환점 Ctrl\+←/)).not.toBeInTheDocument()
    // 소제목 순서: 재생 → 자막 → 장면전환점 → 컬렉션 연속 재생 → 단축키
    const titles = within(page)
      .getAllByRole("heading", { level: 3 })
      .map((h) => h.textContent)
    expect(titles).toEqual(["재생", "자막", "장면전환점", "컬렉션 연속 재생", "단축키"])
  })

  it("T-ui-sreorg-07 [Business rule] the key-conflict warning appears once at the top of 단축키 when the preset and hide shortcut overlap", async () => {
    // Arrange: 숨기기 키 I + Premiere 프리셋(In 키 I)
    const { page } = await setup("플레이어", { hideShortcut: "I", playRangeKeys: "premiere" })
    const shortcuts = await within(page).findByRole("region", { name: "단축키" })
    // Assert
    const warnings = within(page).getAllByText(/숨기기 단축키와 I 키가 겹칩니다/)
    expect(warnings).toHaveLength(1)
    expect(shortcuts).toContainElement(warnings[0])
    expect(warnings[0]).toHaveAttribute("role", "status")
    expect(within(shortcuts).getByRole("radiogroup", { name: "재생 구간 단축키" })).toHaveAttribute("aria-describedby", warnings[0].id)
  })

  it("T-ui-sreorg-08 [Boundary] no warning when the keys do not overlap (default hide key, brackets preset)", async () => {
    // Arrange / Act
    const { page } = await setup("플레이어")
    await within(page).findByRole("region", { name: "단축키" })
    // Assert
    expect(within(page).queryByText(/키가 겹칩니다/)).not.toBeInTheDocument()
    expect(within(page).getByRole("radiogroup", { name: "재생 구간 단축키" })).not.toHaveAttribute("aria-invalid")
  })
})
