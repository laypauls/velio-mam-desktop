// Categories: Happy (T-ui-placement-01, 02, 11) / Boundary (T-ui-placement-03 sidebar closed, 09 viewer closed, 10 missing/unknown → left) / Business rule (T-ui-placement-04, 05 border side, 06, 07 TopBar and rail position, 12 rail/status layout) / Concurrency (T-ui-placement-08 no TopBar remount)
import { act, fireEvent, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { ViewerPlacement } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** a 가 b 보다 문서에서 앞인가 */
const before = (a: Element, b: Element) => Boolean(a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING)

/** TopBar — 보기 영역 헤더도 banner 역할이라 role 대신 test id 로 찾는다 */
const topBar = (): HTMLElement => screen.getByTestId("topbar")
/** 본문 행 — 섹션 패널·보기 영역·갤러리·상세 패널의 부모 */
const bodyRow = (): HTMLElement => screen.getByRole("main").parentElement as HTMLElement

// 폭이 충분한 창(1600) — 섹션 패널 자동 접힘(폭 부족)에 걸리지 않게 한다
const WIDE = { width: 1600 }

async function mount(placement: ViewerPlacement, { sidebar = true }: { sidebar?: boolean } = {}) {
  stubLayout(WIDE)
  const api = createMockApi({ count: 20 })
  await api.updateSettings({ viewerPlacement: placement })
  renderApp(api)
  await screen.findByRole("listbox", { name: "미디어 목록" })
  act(() => useViewStore.setState({ sidebarOpen: sidebar }))
  act(() => useViewStore.getState().openPreview(1))
  const viewer = await screen.findByRole("complementary", { name: "콘텐츠 보기" })
  return {
    viewer,
    handle: screen.getByRole("separator", { name: "보기 영역 폭 조절" }),
    gallery: screen.getByRole("main"),
    detail: screen.getByRole("complementary", { name: "상세 정보 패널" }),
    sidebar: screen.queryByRole("navigation", { name: "라이브러리 탐색" }),
  }
}

describe("AppShell viewer placement", () => {
  it("T-ui-placement-10 [Boundary] missing or unknown viewerPlacement from the backend falls back to left (Sidebar | Viewer | Gallery)", async () => {
    // Arrange: 설정 응답에서 viewerPlacement 가 빠졌거나 알 수 없는 값
    for (const bogus of [undefined, "center"]) {
      stubLayout(WIDE)
      const api = createMockApi({ count: 20 })
      const settings = await api.getSettings()
      vi.spyOn(api, "getSettings").mockResolvedValue({ ...settings, viewerPlacement: bogus as never })
      // Act
      const { unmount } = renderApp(api)
      await screen.findByRole("listbox", { name: "미디어 목록" })
      act(() => useViewStore.setState({ sidebarOpen: true }))
      act(() => useViewStore.getState().openPreview(1))
      const viewer = await screen.findByRole("complementary", { name: "콘텐츠 보기" })
      // Assert: D5 — 패널이 레일에 붙고, 그 오른쪽에 보기 영역, 다음이 갤러리
      const sidebar = screen.getByRole("navigation", { name: "라이브러리 탐색" })
      expect(before(sidebar, viewer)).toBe(true)
      expect(before(viewer, screen.getByRole("main"))).toBe(true)
      unmount()
    }
  })

  it("T-ui-placement-11 [Happy] fresh mock settings (nothing chosen) place the viewer on the left of the gallery", async () => {
    // Arrange + Act
    stubLayout(WIDE)
    const api = createMockApi({ count: 20 })
    renderApp(api)
    await screen.findByRole("listbox", { name: "미디어 목록" })
    act(() => useViewStore.getState().openPreview(1))
    const viewer = await screen.findByRole("complementary", { name: "콘텐츠 보기" })
    // Assert
    expect(before(viewer, screen.getByRole("main"))).toBe(true)
  })

  it("T-ui-placement-01 [Happy] explicit right: Sidebar | Gallery | handle | Viewer | Detail", async () => {
    // Arrange + Act
    const { viewer, handle, gallery, detail, sidebar } = await mount("right")
    // Assert
    expect(sidebar).not.toBeNull()
    expect(before(sidebar as HTMLElement, gallery)).toBe(true)
    expect(before(gallery, handle)).toBe(true)
    expect(before(handle, viewer)).toBe(true)
    expect(before(viewer, detail)).toBe(true)
  })

  it("T-ui-placement-02 [Happy] left: Sidebar | Viewer | handle | Gallery | Detail (detail stays right, panel hugs the rail)", async () => {
    // Arrange + Act
    const { viewer, handle, gallery, detail, sidebar } = await mount("left")
    // Assert
    expect(before(sidebar as HTMLElement, viewer)).toBe(true)
    expect(before(viewer, handle)).toBe(true)
    expect(before(handle, gallery)).toBe(true)
    expect(before(gallery, detail)).toBe(true)
  })

  it("T-ui-placement-03 [Boundary] left with the sidebar closed: Viewer | handle | Gallery | Detail", async () => {
    // Arrange + Act
    const { viewer, handle, gallery, detail, sidebar } = await mount("left", { sidebar: false })
    // Assert
    expect(sidebar).toBeNull()
    expect(before(viewer, handle)).toBe(true)
    expect(before(handle, gallery)).toBe(true)
    expect(before(gallery, detail)).toBe(true)
  })

  it("T-ui-placement-04 [Business rule] the divider border sits on the side facing the gallery (border-l right, border-r left)", async () => {
    // Arrange + Act
    const right = await mount("right")
    // Assert
    expect(right.viewer).toHaveClass("border-l")
    expect(right.viewer).not.toHaveClass("border-r")
  })

  it("T-ui-placement-05 [Business rule] left placement uses border-r instead of border-l", async () => {
    // Arrange + Act
    const { viewer } = await mount("left")
    // Assert
    expect(viewer).toHaveClass("border-r")
    expect(viewer).not.toHaveClass("border-l")
  })

  it.each<ViewerPlacement>(["left", "right"])(
    "T-ui-placement-06 [Business rule] %s: the viewer lives in the body row below the TopBar (never beside it), so the TopBar spans the whole right column",
    async (placement) => {
      // Arrange + Act
      const { viewer, gallery, detail } = await mount(placement)
      const topbar = topBar()
      // Assert: TopBar 와 본문 행은 같은 열의 형제, 보기 영역·갤러리·상세는 모두 본문 행 안
      const row = bodyRow()
      expect(row.parentElement).toBe(topbar.parentElement)
      expect(before(topbar, row)).toBe(true)
      expect(row.contains(viewer)).toBe(true)
      expect(row.contains(gallery)).toBe(true)
      expect(row.contains(detail)).toBe(true)
      expect(topbar.contains(viewer)).toBe(false)
      expect(viewer.contains(topbar)).toBe(false)
    }
  )

  it("T-ui-placement-12 [Business rule] the rail is the first child of the app root and spans the window; the status bar sits in the column to its right, not under the rail", async () => {
    // Arrange + Act
    await mount("left")
    const rail = screen.getByRole("navigation", { name: "주 메뉴" })
    const status = screen.getByRole("status", { name: "작업 상태" })
    // Assert
    expect(rail.parentElement?.firstElementChild).toBe(rail)
    expect(rail.contains(status)).toBe(false)
    expect(before(rail, topBar())).toBe(true)
    expect(before(rail, status)).toBe(true)
    const column = status.parentElement as HTMLElement
    expect(column.parentElement).toBe(rail.parentElement)
    expect(column.contains(topBar())).toBe(true)
  })

  it.each<ViewerPlacement>(["left", "right"])(
    "T-ui-placement-08 [Concurrency] %s: opening/closing the viewer keeps the same TopBar node and the typed search text (no remount)",
    async (placement) => {
      // Arrange: 뷰어 닫힌 상태로 렌더
      stubLayout(WIDE)
      const api = createMockApi({ count: 20 })
      await api.updateSettings({ viewerPlacement: placement })
      renderApp(api)
      await screen.findByRole("listbox", { name: "미디어 목록" })
      const topbar = topBar()
      const parent = topbar.parentElement
      const input = document.querySelector<HTMLInputElement>('input[type="search"]') as HTMLInputElement
      fireEvent.change(input, { target: { value: "sunset" } })
      // Act: 열기
      act(() => useViewStore.getState().openPreview(1))
      await screen.findByRole("complementary", { name: "콘텐츠 보기" })
      // Assert: 같은 노드·같은 부모·같은 첫 자식 위치
      expect(topBar()).toBe(topbar)
      expect(topbar.parentElement).toBe(parent)
      expect(parent?.firstElementChild).toBe(topbar)
      expect(document.querySelector('input[type="search"]')).toBe(input)
      expect(input.value).toBe("sunset")
      // Act: 닫기
      act(() => useViewStore.getState().closePreview())
      // Assert
      expect(screen.queryByRole("complementary", { name: "콘텐츠 보기" })).toBeNull()
      expect(topBar()).toBe(topbar)
      expect(input.isConnected).toBe(true)
      expect(input.value).toBe("sunset")
    }
  )

  it("T-ui-placement-09 [Boundary] viewer closed: no viewer or handle, the body row is just panel | gallery | detail", async () => {
    // Arrange
    stubLayout(WIDE)
    const api = createMockApi({ count: 20 })
    await api.updateSettings({ viewerPlacement: "left" })
    // Act
    renderApp(api)
    await screen.findByRole("listbox", { name: "미디어 목록" })
    // Assert
    expect(screen.queryByRole("complementary", { name: "콘텐츠 보기" })).toBeNull()
    expect(screen.queryByRole("separator", { name: "보기 영역 폭 조절" })).toBeNull()
    expect(topBar().nextElementSibling).toBe(bodyRow())
  })
})
