// Categories: Happy
import { screen, within } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("AppShell", () => {
  it("T-ui-shell-01 [Happy] renders sidebar, top bar and status bar with asset count and scan progress", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 2000, scanning: true })
    // Act
    renderApp(api)
    // Assert
    const nav = screen.getByRole("navigation", { name: "라이브러리 탐색" })
    expect(within(nav).getByRole("button", { name: /전체 미디어/ })).toBeInTheDocument()
    expect(within(nav).getByRole("button", { name: /즐겨찾기/ })).toBeInTheDocument()
    expect(screen.getByRole("combobox", { name: "검색" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "필터" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "설정" })).toBeInTheDocument()
    const status = await screen.findByRole("status", { name: "작업 상태" })
    expect(await within(status).findByText("스캔 D:\\Photos 42%")).toBeInTheDocument()
    expect(within(status).getByRole("button", { name: "작업 일시정지" })).toBeInTheDocument()
    // 관리 폴더 트리: 오프라인 NAS 폴더에 경고 아이콘
    expect(await within(nav).findByText("\\\\NAS\\Archive")).toBeInTheDocument()
    expect(within(nav).getByRole("img", { name: "오프라인" })).toBeInTheDocument()
  })

  it("T-ui-status-01 [Happy] status bar no longer shows the asset count but keeps scan progress and pause control", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 2000, scanning: true })
    // Act
    renderApp(api)
    // Assert
    const status = await screen.findByRole("status", { name: "작업 상태" })
    expect(await within(status).findByText("스캔 D:\\Photos 42%")).toBeInTheDocument()
    expect(within(status).queryByText(/^자산 .*개$/)).not.toBeInTheDocument()
    expect(within(status).getByRole("button", { name: "작업 일시정지" })).toBeInTheDocument()
  })
})
