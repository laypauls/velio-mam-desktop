// Categories: Happy (T-ui-placement-01, 02) / Boundary (T-ui-placement-03 sidebar closed, 04 default before settings load, 10 missing/unknown → left, 11 fresh default left) / Business rule (T-ui-placement-05 border side, 06-07 TopBar tree position, 09 viewer closed) / Concurrency (T-ui-placement-08 no TopBar remount)
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

/** TopBar(`header`) — 보기 영역 헤더도 banner 역할이라 role 대신 오른쪽 열의 직계 자식으로 찾는다 */
const topBar = (): HTMLElement => document.querySelector("[data-viewer-sibling] > header") as HTMLElement

async function mount(placement: ViewerPlacement, { sidebar = true }: { sidebar?: boolean } = {}) {
  stubLayout()
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
  it("T-ui-placement-10 [Boundary] missing or unknown viewerPlacement from the backend falls back to left (Viewer | Sidebar | Gallery)", async () => {
    // Arrange: 설정 응답에서 viewerPlacement 가 빠졌거나 알 수 없는 값
    for (const bogus of [undefined, "center"]) {
      stubLayout()
      const api = createMockApi({ count: 20 })
      const settings = await api.getSettings()
      vi.spyOn(api, "getSettings").mockResolvedValue({ ...settings, viewerPlacement: bogus as never })
      // Act
      const { unmount } = renderApp(api)
      await screen.findByRole("listbox", { name: "미디어 목록" })
      act(() => useViewStore.setState({ sidebarOpen: true }))
      act(() => useViewStore.getState().openPreview(1))
      const viewer = await screen.findByRole("complementary", { name: "콘텐츠 보기" })
      // Assert
      const sidebar = screen.getByRole("navigation", { name: "라이브러리 탐색" })
      expect(before(viewer, sidebar)).toBe(true)
      expect(before(sidebar, screen.getByRole("main"))).toBe(true)
      unmount()
    }
  })

  it("T-ui-placement-11 [Happy] fresh mock settings (nothing chosen) place the viewer on the left", async () => {
    // Arrange + Act
    stubLayout()
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

  it("T-ui-placement-02 [Happy] left: Viewer | handle after the aside | Sidebar | Gallery | Detail (detail stays right)", async () => {
    // Arrange + Act
    const { viewer, handle, gallery, detail, sidebar } = await mount("left")
    // Assert
    expect(before(viewer, handle)).toBe(true)
    expect(before(handle, sidebar as HTMLElement)).toBe(true)
    expect(before(sidebar as HTMLElement, gallery)).toBe(true)
    expect(before(gallery, detail)).toBe(true)
  })

  it("T-ui-placement-03 [Boundary] left with the sidebar closed: Viewer | Gallery | Detail", async () => {
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

  it("T-ui-placement-06 [Business rule] left: TopBar lives in the viewer's sibling column (right of the viewer, above the gallery); StatusBar stays outside both", async () => {
    // Arrange + Act
    const { viewer, gallery } = await mount("left")
    const topbar = topBar()
    // Assert
    const column = topbar.parentElement as HTMLElement
    expect(column).toHaveAttribute("data-viewer-sibling")
    expect(column.parentElement).toBe(viewer.parentElement) // 보기 영역과 같은 행의 형제
    expect(column.contains(viewer)).toBe(false)
    expect(column.contains(gallery)).toBe(true)
    expect(before(viewer, topbar)).toBe(true)
    expect(before(topbar, gallery)).toBe(true)
    expect(viewer.contains(topbar)).toBe(false)
  })

  it("T-ui-placement-07 [Business rule] right placement: TopBar is in the same column, before the gallery and the viewer (unchanged order)", async () => {
    // Arrange + Act
    const { viewer, gallery, detail } = await mount("right")
    const topbar = topBar()
    // Assert
    const column = topbar.parentElement as HTMLElement
    expect(column).toHaveAttribute("data-viewer-sibling")
    expect(column.contains(viewer)).toBe(true) // 오른쪽 배치는 보기 영역도 오른쪽 열 본문 행 안
    expect(column.contains(detail)).toBe(true)
    expect(before(topbar, gallery)).toBe(true)
    expect(before(topbar, viewer)).toBe(true)
  })

  it.each<ViewerPlacement>(["left", "right"])(
    "T-ui-placement-08 [Concurrency] %s: opening/closing the viewer keeps the same TopBar node and the typed search text (no remount)",
    async (placement) => {
      // Arrange: 뷰어 닫힌 상태로 렌더
      stubLayout()
      const api = createMockApi({ count: 20 })
      await api.updateSettings({ viewerPlacement: placement })
      renderApp(api)
      await screen.findByRole("listbox", { name: "미디어 목록" })
      const topbar = topBar()
      const input = document.querySelector<HTMLInputElement>('input[type="search"]') as HTMLInputElement
      fireEvent.change(input, { target: { value: "sunset" } })
      // Act: 열기
      act(() => useViewStore.getState().openPreview(1))
      await screen.findByRole("complementary", { name: "콘텐츠 보기" })
      // Assert
      expect(topBar()).toBe(topbar)
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

  it("T-ui-placement-09 [Boundary] viewer closed: no viewer or handle, TopBar still first in the single full-width column", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 20 })
    await api.updateSettings({ viewerPlacement: "left" })
    // Act
    renderApp(api)
    await screen.findByRole("listbox", { name: "미디어 목록" })
    // Assert
    const topbar = topBar()
    const column = topbar.parentElement as HTMLElement
    expect(screen.queryByRole("complementary", { name: "콘텐츠 보기" })).toBeNull()
    expect(screen.queryByRole("separator", { name: "보기 영역 폭 조절" })).toBeNull()
    expect(column.parentElement?.firstElementChild).toBe(column)
    expect(topbar).toBe(column.firstElementChild)
  })
})
