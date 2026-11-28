// Categories: Happy (T-ui-rail-store-01, 02) / Boundary (T-ui-rail-store-03, 06) / Business rule (T-ui-rail-store-04 auto collapse never touches the saved value, 05 hydrate)
import { beforeEach, describe, expect, it } from "vitest"
import { resetAllStores, useViewStore } from "@/stores"
import { selectPanelVisible } from "./view"

const s = () => useViewStore.getState()

beforeEach(() => resetAllStores())

describe("sidebar sections (left rail)", () => {
  it("T-ui-rail-store-01 [Happy] selecting another section shows it and opens the panel; the section is a layout change (saved)", () => {
    // Arrange
    useViewStore.setState({ sidebarOpen: false })
    const rev = s().layoutRev
    // Act
    s().selectSection("tags")
    // Assert
    expect(s().sidebarTab).toBe("tags")
    expect(s().sidebarOpen).toBe(true)
    expect(selectPanelVisible(s())).toBe(true)
    expect(s().layoutRev).toBeGreaterThan(rev)
  })

  it("T-ui-rail-store-02 [Happy] re-clicking the visible active section collapses the panel, and clicking it again reopens it", () => {
    // Arrange
    s().selectSection("folders")
    // Act + Assert
    s().selectSection("folders")
    expect(s().sidebarOpen).toBe(false)
    expect(s().sidebarTab).toBe("folders")
    s().selectSection("folders")
    expect(s().sidebarOpen).toBe(true)
  })

  it("T-ui-rail-store-03 [Boundary] setSidebarTab with the current value does not bump layoutRev (no needless save); the default is library", () => {
    expect(s().sidebarTab).toBe("library")
    s().setSidebarTab("library")
    expect(s().layoutRev).toBe(0)
    s().setSidebarTab("collections")
    expect(s().layoutRev).toBe(1)
  })

  it("T-ui-rail-store-04 [Business rule] while the panel is auto-collapsed the logo only re-shows it (saved sidebarOpen stays true); the next toggle closes it for real", () => {
    // Arrange: 보기 영역이 열려 폭이 모자란 상태
    s().setPanelAutoCollapsed(true)
    expect(selectPanelVisible(s())).toBe(false)
    expect(s().sidebarOpen).toBe(true)
    const rev = s().layoutRev
    // Act: 로고 — 다시 펼치기
    s().toggleSidebar()
    // Assert: 저장값·레이아웃 저장은 그대로
    expect(selectPanelVisible(s())).toBe(true)
    expect(s().sidebarOpen).toBe(true)
    expect(s().layoutRev).toBe(rev)
    // Act: 한 번 더 — 실제로 접는다
    s().toggleSidebar()
    expect(s().sidebarOpen).toBe(false)
    expect(selectPanelVisible(s())).toBe(false)
  })

  it("T-ui-rail-store-05 [Business rule] a forced panel is released when the auto-collapse verdict flips or the viewer closes (clearPanelForced); a section click while auto-collapsed re-shows that section", () => {
    // Arrange
    s().setPanelAutoCollapsed(true)
    s().selectSection("tags") // 접힌 상태에서 섹션 클릭 → 그 섹션을 다시 보인다(접지 않는다)
    expect(selectPanelVisible(s())).toBe(true)
    expect(s().sidebarTab).toBe("tags")
    // Act: 판정이 바뀜(넓어짐) → 해제
    s().setPanelAutoCollapsed(false)
    expect(s().panelForced).toBe(false)
    // Act: 다시 좁아져 접히고 강제 → 보기 영역 닫힘으로 해제
    s().setPanelAutoCollapsed(true)
    s().toggleSidebar()
    expect(s().panelForced).toBe(true)
    s().clearPanelForced()
    expect(s().panelForced).toBe(false)
    expect(selectPanelVisible(s())).toBe(false)
  })

  it("T-ui-rail-store-06 [Boundary] hydrateLayout restores the section only before the user changed the layout", () => {
    // Arrange + Act
    s().hydrateLayout({ detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104, sidebarSection: "collections" })
    // Assert
    expect(s().sidebarTab).toBe("collections")
    expect(s().layoutRev).toBe(0)
    // Act: 사용자 변경 후에는 저장값이 덮지 않는다
    s().setSidebarTab("tags")
    s().hydrateLayout({ detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104, sidebarSection: "rating" })
    expect(s().sidebarTab).toBe("tags")
  })
})
