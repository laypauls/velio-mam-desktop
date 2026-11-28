// Categories: Happy (T-ui-winctl-01 buttons call the window API, 02 maximize icon sync) / Business rule (T-ui-winctl-03 close() not destroy(), 06 drag region only on non-interactive elements) / External failure (T-ui-winctl-04 rejects are swallowed) / Boundary (T-ui-winctl-05 no Tauri window) / Concurrency (T-ui-winctl-07 unsubscribe)
import { act, render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { resetAllStores } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { WindowControls } from "./WindowControls"

const win = vi.hoisted(() => ({
  maximized: false,
  resizeHandler: null as null | (() => void),
  unlisten: vi.fn(),
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
  destroy: vi.fn(),
  isMaximized: vi.fn(),
  onResized: vi.fn(),
}))
const tauri = vi.hoisted(() => ({ available: true }))
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => {
    if (!tauri.available) throw new TypeError("no tauri")
    return win
  },
}))

beforeEach(() => {
  resetAllStores()
  tauri.available = true
  win.maximized = false
  win.resizeHandler = null
  win.unlisten.mockReset()
  win.minimize.mockReset().mockResolvedValue(undefined)
  win.toggleMaximize.mockReset().mockResolvedValue(undefined)
  win.close.mockReset().mockResolvedValue(undefined)
  win.destroy.mockReset().mockResolvedValue(undefined)
  win.isMaximized.mockReset().mockImplementation(async () => win.maximized)
  win.onResized.mockReset().mockImplementation(async (h: () => void) => {
    win.resizeHandler = h
    return win.unlisten
  })
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const group = () => screen.getByRole("group", { name: "창 조절" })

describe("WindowControls", () => {
  it("T-ui-winctl-01 [Happy] minimize, maximize and close call the matching window API once each", async () => {
    // Arrange
    const user = userEvent.setup()
    render(<WindowControls />)
    // Act
    await user.click(within(group()).getByRole("button", { name: "최소화" }))
    await user.click(within(group()).getByRole("button", { name: "최대화" }))
    await user.click(within(group()).getByRole("button", { name: "닫기" }))
    // Assert
    expect(win.minimize).toHaveBeenCalledTimes(1)
    expect(win.toggleMaximize).toHaveBeenCalledTimes(1)
    expect(win.close).toHaveBeenCalledTimes(1)
  })

  it("T-ui-winctl-03 [Business rule] close goes through close() so the tray-close path in tray.rs runs; destroy() is never used", async () => {
    // Arrange
    const user = userEvent.setup()
    render(<WindowControls />)
    // Act
    await user.click(within(group()).getByRole("button", { name: "닫기" }))
    // Assert
    expect(win.close).toHaveBeenCalledTimes(1)
    expect(win.destroy).not.toHaveBeenCalled()
  })

  it("T-ui-winctl-02 [Happy] the maximize button label follows the real window state via resize events (Win+Up, snap, double-click)", async () => {
    // Arrange: 시작은 일반 상태
    render(<WindowControls />)
    expect(await within(group()).findByRole("button", { name: "최대화" })).toBeInTheDocument()
    // Act: 창 밖에서 최대화됨 → 크기 변경 이벤트
    win.maximized = true
    await act(async () => win.resizeHandler?.())
    // Assert
    expect(await within(group()).findByRole("button", { name: "이전 크기로 복원" })).toBeInTheDocument()
    expect(within(group()).queryByRole("button", { name: "최대화" })).not.toBeInTheDocument()
    // Act: 복원
    win.maximized = false
    await act(async () => win.resizeHandler?.())
    // Assert
    expect(await within(group()).findByRole("button", { name: "최대화" })).toBeInTheDocument()
  })

  it("T-ui-winctl-02b [Happy] a window that starts maximized shows the restore label right away", async () => {
    // Arrange
    win.maximized = true
    // Act
    render(<WindowControls />)
    // Assert
    expect(await within(group()).findByRole("button", { name: "이전 크기로 복원" })).toBeInTheDocument()
  })

  it("T-ui-winctl-04 [External] a rejected window call (denied capability etc.) does not throw to the UI", async () => {
    // Arrange
    const user = userEvent.setup()
    win.close.mockRejectedValue(new Error("denied"))
    win.minimize.mockRejectedValue(new Error("denied"))
    win.isMaximized.mockRejectedValue(new Error("denied"))
    render(<WindowControls />)
    // Act
    await user.click(within(group()).getByRole("button", { name: "닫기" }))
    await user.click(within(group()).getByRole("button", { name: "최소화" }))
    // Assert: 여전히 조작 가능
    expect(group()).toBeInTheDocument()
    expect(within(group()).getByRole("button", { name: "최대화" })).toBeInTheDocument()
  })

  it("T-ui-winctl-05 [Boundary] without a Tauri window (browser/mock) the controls render and clicks are no-ops", async () => {
    // Arrange
    tauri.available = false
    const user = userEvent.setup()
    // Act
    render(<WindowControls />)
    await user.click(within(group()).getByRole("button", { name: "닫기" }))
    // Assert
    expect(win.close).not.toHaveBeenCalled()
    expect(within(group()).getAllByRole("button")).toHaveLength(3)
  })

  it("T-ui-winctl-07 [Concurrency] unmounting unsubscribes from resize events, even when the subscription resolves late", async () => {
    // Arrange
    const { unmount } = render(<WindowControls />)
    await act(async () => undefined)
    // Act
    unmount()
    // Assert
    expect(win.unlisten).toHaveBeenCalledTimes(1)
  })
})

describe("TopBar as the window title bar", () => {
  it("T-ui-winctl-06 [Business rule] the drag region sits on the header, its inner wrapper and the stretching spacer only — never on search, filter or buttons — and the window buttons are inside the header", async () => {
    // Arrange + Act
    stubLayout()
    renderApp(createMockApi({ count: 5 }))
    const header = await screen.findByTestId("topbar")
    // Assert
    expect(header).toHaveAttribute("data-tauri-drag-region")
    const regions = Array.from(header.querySelectorAll("[data-tauri-drag-region]"))
    expect(regions.length).toBeGreaterThanOrEqual(2) // 래퍼 + 빈 칸
    for (const el of [...regions, header]) {
      expect(el.closest("button, input, [role=combobox], [role=button]")).toBeNull()
      expect(el.matches("button, input, select, textarea, a")).toBe(false)
    }
    for (const control of Array.from(header.querySelectorAll("button, input"))) {
      expect(control).not.toHaveAttribute("data-tauri-drag-region")
    }
    expect(within(header).getByRole("group", { name: "창 조절" })).toBeInTheDocument()
    // 이전 ☰·⚙ 는 TopBar 에 없다
    expect(within(header).queryByRole("button", { name: "설정" })).not.toBeInTheDocument()
    expect(within(header).queryByRole("button", { name: "사이드바 열기/닫기" })).not.toBeInTheDocument()
  })
})
