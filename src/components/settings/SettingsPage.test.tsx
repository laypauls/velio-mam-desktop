// Categories: Happy (T-ui-spage-01~03) / Business rule (T-ui-spage-04~06) / Boundary + Validation (T-ui-spage-05, T-ui-spage-07)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useProtectStore, useSearchStore, useSelectionStore, type SettingsSection } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { openFolderSettings } from "@/test/folderDialog"
import { openSettingsPage } from "@/test/settings"
import { SETTINGS_MENU } from "./settingsMenu"

// 메타데이터 필드 탭은 Tauri command 를 직접 부른다 — 메뉴를 순회하는 시험이 jsdom 에서 실제 invoke 를 부르지 않게 한다
vi.mock("@/lib/ipc-metadata", () => ({ listMetadataFields: () => Promise.resolve([]) }))
vi.mock("@/lib/ipc-plugin", () => ({
  pluginList: () => Promise.resolve([]),
  pluginTaskCounts: () => Promise.resolve([]),
  onPluginChanged: () => Promise.resolve(() => undefined),
}))

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const LABELS: Record<SettingsSection, string> = {
  general: "일반",
  view: "탐색·표시",
  player: "플레이어",
  resume: "이어보기",
  folders: "관리 폴더",
  metadata: "메타데이터 필드",
  ai: "외부 AI",
  "ai-local": "로컬 AI·검색",
  protection: "보호",
  data: "데이터·백업",
  plugins: "플러그인",
  licenses: "오픈소스 라이선스",
}

async function menu() {
  return within(await screen.findByRole("navigation", { name: "설정 메뉴" }))
}

describe("settings page — menu and entry points", () => {
  it("T-ui-spage-01 [Happy] menu order follows the definition array and each page shows an h2 title + description", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    // Act
    await openSettingsPage(user)
    // Assert: 배열 순서 = 메뉴 순서, 첫 진입은 일반
    const nav = await menu()
    const names = nav.getAllByRole("button").map((b) => b.textContent)
    expect(names).toEqual(["일반", "탐색·표시", "플레이어", "이어보기", "관리 폴더", "메타데이터 필드", "외부 AI", "로컬 AI·검색", "보호", "데이터·백업", "플러그인", "오픈소스 라이선스"])
    expect(SETTINGS_MENU.map((m) => m.id)).toEqual(["general", "view", "player", "resume", "folders", "metadata", "ai", "ai-local", "protection", "data", "plugins", "licenses"])
    expect(nav.getByRole("button", { name: "일반" })).toHaveAttribute("aria-current", "page")
    // 모든 메뉴: h2 제목 + 설명 1줄
    for (const item of SETTINGS_MENU) {
      await user.click(nav.getByRole("button", { name: LABELS[item.id] }))
      expect(screen.getByRole("heading", { level: 2, name: LABELS[item.id] })).toBeInTheDocument()
      expect(nav.getByRole("button", { name: LABELS[item.id] })).toHaveAttribute("aria-current", "page")
      expect(nav.getAllByRole("button").filter((b) => b.hasAttribute("aria-current"))).toHaveLength(1)
    }
  })

  it("T-ui-spage-02 [Happy] 탐색·표시 is its own page and is no longer inside 일반", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    // Act + Assert: 일반
    const main = await openSettingsPage(user)
    await within(main).findByRole("switch", { name: "창을 닫으면 트레이로 보내기" })
    expect(within(main).queryByRole("group", { name: "정렬 항목" })).not.toBeInTheDocument()
    // 탐색·표시
    await user.click((await menu()).getByRole("button", { name: "탐색·표시" }))
    expect(await within(main).findByRole("group", { name: "정렬 항목" })).toBeInTheDocument()
    expect(within(main).queryByRole("switch", { name: "창을 닫으면 트레이로 보내기" })).not.toBeInTheDocument()
  })

  it.each<[SettingsSection | undefined, SettingsSection]>([
    ["ai", "ai"],
    ["ai-local", "ai-local"],
    ["data", "data"],
    ["folders", "folders"],
    ["protection", "protection"],
    ["view", "view"],
    ["player", "player"],
    [undefined, "general"],
  ])("T-ui-spage-03 [Happy] openSettings(%s) lands on the %s page", async (arg, expected) => {
    // Arrange
    stubLayout()
    renderApp(createMockApi({ count: 50 }))
    // Act
    act(() => useProtectStore.getState().openSettings(arg))
    // Assert
    const nav = await menu()
    expect(nav.getByRole("button", { name: LABELS[expected] })).toHaveAttribute("aria-current", "page")
    expect(await screen.findByRole("heading", { level: 2, name: LABELS[expected] })).toBeInTheDocument()
  })

  it("T-ui-spage-04 [Business rule] openSettings() without a section reopens the last viewed section", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    await openSettingsPage(user, "데이터·백업")
    // Act: 복귀 후 다시 열기
    await user.click(screen.getByRole("button", { name: "라이브러리로" }))
    await user.click(screen.getByRole("button", { name: "설정" }))
    // Assert
    expect((await menu()).getByRole("button", { name: "데이터·백업" })).toHaveAttribute("aria-current", "page")
  })

  it("T-ui-spage-05 [Boundary] ArrowDown/ArrowUp move between items, wrap at both ends and move focus with the selection", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    await openSettingsPage(user)
    const nav = await menu()
    nav.getByRole("button", { name: "일반" }).focus()
    // Act + Assert: 아래
    await user.keyboard("{ArrowDown}")
    expect(nav.getByRole("button", { name: "탐색·표시" })).toHaveAttribute("aria-current", "page")
    expect(nav.getByRole("button", { name: "탐색·표시" })).toHaveFocus()
    // 위로 두 번 → 처음에서 끝으로 순환
    await user.keyboard("{ArrowUp}{ArrowUp}")
    expect(nav.getByRole("button", { name: "오픈소스 라이선스" })).toHaveAttribute("aria-current", "page")
    expect(nav.getByRole("button", { name: "오픈소스 라이선스" })).toHaveFocus()
    // 끝에서 아래 → 처음
    await user.keyboard("{ArrowDown}")
    expect(nav.getByRole("button", { name: "일반" })).toHaveAttribute("aria-current", "page")
    // 다른 키는 무시
    await user.keyboard("{ArrowRight}")
    expect(nav.getByRole("button", { name: "일반" })).toHaveAttribute("aria-current", "page")
  })

  it("T-ui-spage-06 [Business rule] the settings screen keeps the status bar and has a single main landmark; library is hidden from the a11y tree", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50, scanning: true }))
    // Act
    const main = await openSettingsPage(user)
    // Assert
    expect(await screen.findByRole("status", { name: "작업 상태" })).toBeInTheDocument()
    expect(screen.getAllByRole("main")).toEqual([main])
    expect(screen.queryByRole("combobox", { name: "검색" })).not.toBeInTheDocument()
  })
})

describe("settings page — returning to the library", () => {
  it("T-ui-spage-07 [Happy] back button and Esc return to the library; Esc is ignored while an inner alertdialog is open", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    await api.updateSettings({ hideShortcut: null }) // 기본 Esc 는 숨기기 단축키 — 설정 닫기 Esc 는 단축키가 다른 키일 때 돌아온다
    const removeSpy = vi.spyOn(api, "removeFolder")
    const user = userEvent.setup()
    renderApp(api)
    const main = await openSettingsPage(user, "관리 폴더")
    // Act 1: 하위 AlertDialog 를 연 뒤 Esc → AlertDialog 만 닫힌다
    const folderDialog = await openFolderSettings(user, main, "D:\\Photos")
    await user.click(within(folderDialog).getByRole("button", { name: "폴더 제거" }))
    const alert = await screen.findByRole("alertdialog")
    await user.keyboard("{Escape}")
    // Assert 1
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument())
    expect(screen.getByRole("navigation", { name: "설정 메뉴", hidden: true })).toBeInTheDocument() // 폴더 설정 다이얼로그가 남아 aria-hidden
    expect(alert).not.toBeInTheDocument()
    expect(removeSpy).not.toHaveBeenCalled()
    // Act 1b: 폴더 설정 다이얼로그가 남아 있으면 Esc → 그 다이얼로그만 닫힌다
    expect(screen.getByRole("dialog", { name: "관리 폴더 설정" })).toBeInTheDocument()
    await user.keyboard("{Escape}")
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument())
    expect(screen.getByRole("navigation", { name: "설정 메뉴" })).toBeInTheDocument()
    // Act 2: 열린 레이어가 없으면 Esc → 라이브러리
    await user.keyboard("{Escape}")
    // Assert 2
    await waitFor(() => expect(screen.queryByRole("navigation", { name: "설정 메뉴" })).not.toBeInTheDocument())
    expect(screen.getByRole("combobox", { name: "검색" })).toBeInTheDocument()
    // Act 3: 다시 열고 ← 버튼
    await user.click(screen.getByRole("button", { name: "설정" }))
    await user.click(await screen.findByRole("button", { name: "라이브러리로" }))
    // Assert 3
    await waitFor(() => expect(screen.queryByRole("navigation", { name: "설정 메뉴" })).not.toBeInTheDocument())
    expect(useProtectStore.getState().settingsOpen).toBe(false)
  })

  it("T-ui-spage-08 [Business rule] library state (search text, selection) survives a settings round trip without remounting", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 300 })
    await api.updateSettings({ hideShortcut: null }) // 설정 닫기 Esc 를 쓰려고 숨기기 단축키를 끈다
    renderApp(api)
    await screen.findAllByRole("option")
    const box = screen.getByRole("combobox", { name: "검색" })
    await user.type(box, "abc")
    // 디바운스된 질의가 반영(= 검색 조건 변경으로 선택이 비워지는 시점)된 뒤에 선택한다
    await waitFor(() => expect(useSearchStore.getState().query).toBe("abc"))
    await screen.findAllByRole("option")
    act(() => useSelectionStore.getState().selectAll([1, 2]))
    // Act: 설정 왕복
    await openSettingsPage(user, "외부 AI")
    // 설정 중에도 라이브러리 DOM 은 그대로(숨김) — 같은 요소, 값 유지
    expect(screen.getByRole("combobox", { name: "검색", hidden: true })).toBe(box)
    await user.keyboard("{Escape}")
    // Assert
    await waitFor(() => expect(screen.getByRole("combobox", { name: "검색" })).toBe(box))
    expect(box).toHaveValue("abc")
    expect([...useSelectionStore.getState().ids]).toEqual([1, 2])
    expect(box.closest("[inert]")).toBeNull()
  })

  it("T-ui-spage-09 [Business rule] an open preview is closed when settings opens so it cannot cover the settings page", async () => {
    // Arrange
    stubLayout()
    renderApp(createMockApi({ count: 50 }))
    const { useViewStore } = await import("@/stores")
    act(() => useViewStore.getState().openPreview(1))
    // Act
    act(() => useProtectStore.getState().openSettings("ai"))
    // Assert
    await waitFor(() => expect(useViewStore.getState().previewId).toBeNull())
    expect(await screen.findByRole("navigation", { name: "설정 메뉴" })).toBeInTheDocument()
  })
})
