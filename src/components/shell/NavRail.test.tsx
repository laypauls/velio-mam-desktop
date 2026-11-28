// Categories: Happy (T-ui-rail-01 order, 02 section switch, 04 logo toggle) / Boundary (T-ui-rail-03 re-click collapses, 09 auto collapse) / Business rule (T-ui-rail-05 rating gate OFF, 06 current-view dot, 07 settings entry, 08 tag filter dot, 10 a11y wiring)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useProtectStore, useSearchStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const rail = () => screen.getByRole("navigation", { name: "주 메뉴" })
const railButton = (name: string) => within(rail()).getByRole("button", { name })
const panel = () => screen.queryByRole("navigation", { name: "라이브러리 탐색" })
const WIDE = { width: 1600 }

async function setup(opts: { ratingMenu?: boolean } = {}) {
  stubLayout(WIDE)
  const user = userEvent.setup()
  const api = createMockApi({ count: 50 })
  if (opts.ratingMenu === false) await api.updateSettings({ sidebarRatingMenu: false })
  renderApp(api)
  await screen.findByRole("listbox", { name: "미디어 목록" })
  return { user, api }
}

describe("NavRail", () => {
  it("T-ui-rail-01 [Happy] the rail lists logo, the five sections and settings last, and is always present", async () => {
    // Arrange + Act
    await setup()
    // Assert: 로고 → 섹션(라이브러리·별점·관리 폴더·태그·컬렉션) → 설정
    expect(within(rail()).getAllByRole("button").map((b) => b.getAttribute("aria-label"))).toEqual([
      "사이드바 숨기기/표시",
      "라이브러리",
      "별점",
      "관리 폴더",
      "태그",
      "컬렉션",
      "설정",
    ])
    expect(rail().contains(screen.getByRole("status", { name: "작업 상태" }))).toBe(false)
  })

  it("T-ui-rail-02 [Happy] a section button shows exactly that section's panel and marks itself expanded", async () => {
    // Arrange
    const { user } = await setup()
    expect(railButton("라이브러리")).toHaveAttribute("aria-expanded", "true")
    expect(within(panel() as HTMLElement).getByRole("region", { name: "라이브러리" })).toBeInTheDocument()
    // Act
    await user.click(railButton("컬렉션"))
    // Assert
    expect(await within(panel() as HTMLElement).findByRole("button", { name: /^포트폴리오/ })).toBeInTheDocument()
    expect(within(panel() as HTMLElement).queryByRole("region", { name: "라이브러리" })).not.toBeInTheDocument()
    expect(railButton("컬렉션")).toHaveAttribute("aria-expanded", "true")
    expect(railButton("라이브러리")).toHaveAttribute("aria-expanded", "false")
    expect(useViewStore.getState().sidebarTab).toBe("collections")
  })

  it("T-ui-rail-03 [Boundary] re-clicking the active section collapses the panel (saved closed), and clicking it again reopens", async () => {
    // Arrange
    const { user } = await setup()
    // Act
    await user.click(railButton("라이브러리"))
    // Assert
    expect(panel()).not.toBeInTheDocument()
    expect(useViewStore.getState().sidebarOpen).toBe(false)
    expect(railButton("라이브러리")).toHaveAttribute("aria-expanded", "false")
    // Act: 접힌 상태에서도 섹션 전환 가능
    await user.click(railButton("태그"))
    // Assert
    expect(panel()).toBeInTheDocument()
    expect(railButton("태그")).toHaveAttribute("aria-expanded", "true")
  })

  it("T-ui-rail-04 [Happy] the logo toggles the panel exactly like the old menu button (same store action, section kept)", async () => {
    // Arrange
    const { user } = await setup()
    await user.click(railButton("관리 폴더"))
    const logo = railButton("사이드바 숨기기/표시")
    expect(logo).toHaveAttribute("aria-expanded", "true")
    // Act
    await user.click(logo)
    // Assert
    expect(panel()).not.toBeInTheDocument()
    expect(logo).toHaveAttribute("aria-expanded", "false")
    expect(useViewStore.getState().sidebarOpen).toBe(false)
    // Act
    await user.click(logo)
    // Assert: 같은 섹션이 다시 나타난다
    expect(within(panel() as HTMLElement).getByRole("region", { name: "관리 폴더" })).toBeInTheDocument()
    expect(useViewStore.getState().sidebarTab).toBe("folders")
  })

  it("T-ui-rail-05 [Business rule, gate OFF] with the rating menu setting off the rating icon is hidden and a stored rating section falls back to library", async () => {
    // Arrange: 마지막 섹션이 별점이던 사용자가 설정을 껐다
    stubLayout(WIDE)
    const api = createMockApi({ count: 50 })
    await api.updateSettings({ sidebarRatingMenu: false, layout: { detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104, sidebarSection: "rating" } })
    // Act
    renderApp(api)
    await screen.findByRole("listbox", { name: "미디어 목록" })
    // Assert
    await waitFor(() => expect(useViewStore.getState().sidebarTab).toBe("library"))
    expect(within(rail()).queryByRole("button", { name: "별점" })).not.toBeInTheDocument()
    expect(within(panel() as HTMLElement).getByRole("region", { name: "라이브러리" })).toBeInTheDocument()
    expect(railButton("라이브러리")).toHaveAttribute("aria-expanded", "true")
  })

  it("T-ui-rail-05b [Business rule, gate ON] with the setting on, a stored rating section is restored after startup", async () => {
    // Arrange
    stubLayout(WIDE)
    const api = createMockApi({ count: 50 })
    await api.updateSettings({ layout: { detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104, sidebarSection: "rating" } })
    // Act
    renderApp(api)
    // Assert
    expect(await within(await screen.findByRole("navigation", { name: "라이브러리 탐색" })).findByRole("region", { name: "별점" })).toBeInTheDocument()
    expect(railButton("별점")).toHaveAttribute("aria-expanded", "true")
  })

  it("T-ui-rail-06 [Business rule] the section owning the current gallery view gets a dot only while another section is shown or the panel is collapsed", async () => {
    // Arrange
    const { user } = await setup()
    expect(within(rail()).queryByTestId("rail-view-dot")).not.toBeInTheDocument() // 라이브러리 보기 + 라이브러리 패널
    // Act: 폴더 보기로 이동한 뒤 다른 섹션을 연다
    act(() => useSearchStore.getState().setView({ kind: "folder", folderId: 1, relPath: null }))
    // Assert: 패널은 라이브러리 → 폴더 아이콘에 점
    expect(within(railButton("관리 폴더")).getByTestId("rail-view-dot")).toBeInTheDocument()
    expect(within(railButton("라이브러리")).queryByTestId("rail-view-dot")).not.toBeInTheDocument()
    // Act: 폴더 섹션을 보면 점이 사라진다
    await user.click(railButton("관리 폴더"))
    expect(within(rail()).queryByTestId("rail-view-dot")).not.toBeInTheDocument()
    // Act: 패널을 접으면 다시 나타난다
    await user.click(railButton("사이드바 숨기기/표시"))
    expect(within(railButton("관리 폴더")).getByTestId("rail-view-dot")).toBeInTheDocument()
    expect(railButton("관리 폴더")).toHaveAccessibleDescription("현재 보기")
  })

  it("T-ui-rail-08 [Business rule] an active tag filter puts the dot on the tags section", async () => {
    // Arrange
    await setup()
    // Act
    act(() => useSearchStore.getState().patchFilters({ tagIds: [1] }))
    // Assert
    expect(within(railButton("태그")).getByTestId("rail-view-dot")).toBeInTheDocument()
    // Act: 필터 해제
    act(() => useSearchStore.getState().patchFilters({ tagIds: [] }))
    expect(within(railButton("태그")).queryByTestId("rail-view-dot")).not.toBeInTheDocument()
  })

  it("T-ui-rail-07 [Business rule] the rail settings button is the settings entry: toggles settings (aria-pressed), and a section click closes settings and goes to that section", async () => {
    // Arrange
    const { user } = await setup()
    expect(railButton("설정")).toHaveAttribute("aria-pressed", "false")
    // Act
    await user.click(railButton("설정"))
    // Assert: 설정이 열리고 레일·상태 표시줄은 그대로
    expect(useProtectStore.getState().settingsOpen).toBe(true)
    expect(railButton("설정")).toHaveAttribute("aria-pressed", "true")
    expect(rail()).not.toHaveAttribute("inert")
    expect(screen.getByRole("status", { name: "작업 상태" })).toBeInTheDocument()
    // Act: 섹션 클릭
    await user.click(railButton("태그"))
    // Assert
    expect(useProtectStore.getState().settingsOpen).toBe(false)
    expect(useViewStore.getState().sidebarTab).toBe("tags")
    expect(useViewStore.getState().sidebarOpen).toBe(true)
    expect(railButton("설정")).toHaveAttribute("aria-pressed", "false")
    // Act: 설정을 다시 눌러 닫기
    await user.click(railButton("설정"))
    await user.click(railButton("설정"))
    expect(useProtectStore.getState().settingsOpen).toBe(false)
  })

  it("T-ui-rail-09 [Boundary] with a viewer open in a narrow window the panel auto-collapses without changing the saved value; the logo re-shows it; closing the viewer releases it", async () => {
    // Arrange: 본문 행 폭 1000 < 패널 240 + 보기 320 + 6 + 갤러리 288 + 상세 320
    stubLayout({ width: 1000 })
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    await screen.findByRole("listbox", { name: "미디어 목록" })
    expect(panel()).toBeInTheDocument()
    // Act: 보기 영역 열기
    act(() => useViewStore.getState().openPreview(1))
    await screen.findByRole("complementary", { name: "콘텐츠 보기" })
    // Assert
    expect(panel()).not.toBeInTheDocument()
    expect(useViewStore.getState().sidebarOpen).toBe(true)
    expect(railButton("사이드바 숨기기/표시")).toHaveAttribute("aria-expanded", "false")
    expect(screen.getByRole("complementary", { name: "상세 정보 패널" })).toBeInTheDocument() // 상세 패널은 자동으로 닫지 않는다
    // Act: 로고로 다시 펼치기 — 사용자 선택 존중, 저장값은 그대로
    const rev = useViewStore.getState().layoutRev
    await user.click(railButton("사이드바 숨기기/표시"))
    expect(panel()).toBeInTheDocument()
    expect(useViewStore.getState().layoutRev).toBe(rev)
    // Act: 보기 영역을 닫으면 존중 상태 해제
    act(() => useViewStore.getState().closePreview())
    expect(panel()).toBeInTheDocument()
    expect(useViewStore.getState().panelForced).toBe(false)
    act(() => useViewStore.getState().openPreview(2))
    await screen.findByRole("complementary", { name: "콘텐츠 보기" })
    expect(panel()).not.toBeInTheDocument()
  })

  it("T-ui-rail-10 [Business rule] a11y wiring: nav landmark, section buttons control the panel id, every rail button has an accessible name", async () => {
    // Arrange + Act
    await setup()
    const id = (panel() as HTMLElement).id
    // Assert
    expect(id).not.toBe("")
    expect(railButton("컬렉션")).toHaveAttribute("aria-controls", id)
    expect(railButton("사이드바 숨기기/표시")).toHaveAttribute("aria-controls", id)
    for (const b of within(rail()).getAllByRole("button")) expect(b).toHaveAccessibleName()
  })

  it("T-ui-rail-11 [Regression] with the panel open the ghost variant's aria-expanded background is neutralised for the logo and overridden for the active section", async () => {
    // jsdom 은 변형 CSS 를 계산하지 못해 클래스 계약으로 검증한다(tests.md §5 예외) — 실제 렌더는 tauri dev 에서 확인
    // Arrange + Act
    await setup()
    // Assert
    const logo = railButton("사이드바 숨기기/표시")
    expect(logo).toHaveAttribute("aria-expanded", "true")
    expect(logo.className).toContain("aria-expanded:bg-transparent")
    expect(logo.className).not.toContain("aria-expanded:bg-muted")
    const active = railButton("라이브러리")
    expect(active).toHaveAttribute("aria-expanded", "true")
    expect(active.className).toContain("aria-expanded:bg-sidebar-accent")
    expect(active.className).not.toContain("aria-expanded:bg-transparent")
    expect(active.className).not.toContain("aria-expanded:bg-muted")
  })
})
