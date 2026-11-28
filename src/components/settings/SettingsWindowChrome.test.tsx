// Categories: Happy (T-ui-winctl-10 settings header window buttons) / Business rule (T-ui-winctl-11 drag region + same chrome height token, 12 settings covers only the stage)
import { screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"

const win = vi.hoisted(() => ({
  close: vi.fn(),
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  isMaximized: vi.fn(),
  onResized: vi.fn(),
}))
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ ...win, setTheme: vi.fn().mockResolvedValue(undefined) }) }))

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function openSettings() {
  stubLayout()
  win.close.mockReset().mockResolvedValue(undefined)
  win.minimize.mockReset().mockResolvedValue(undefined)
  win.toggleMaximize.mockReset().mockResolvedValue(undefined)
  win.isMaximized.mockReset().mockResolvedValue(false)
  win.onResized.mockReset().mockResolvedValue(() => undefined)
  const user = userEvent.setup()
  renderApp(createMockApi({ count: 10 }))
  await screen.findByRole("listbox", { name: "미디어 목록" })
  await user.click(within(screen.getByRole("navigation", { name: "주 메뉴" })).getByRole("button", { name: "설정" }))
  const title = await screen.findByRole("heading", { name: "설정", level: 1 })
  return { user, header: title.closest("header") as HTMLElement }
}

describe("settings header as window chrome", () => {
  it("T-ui-winctl-10 [Happy] the settings header carries the window buttons; close goes through close()", async () => {
    // Arrange
    const { user, header } = await openSettings()
    // Act
    await user.click(within(header).getByRole("button", { name: "최소화" }))
    await user.click(within(header).getByRole("button", { name: "닫기" }))
    // Assert
    expect(win.minimize).toHaveBeenCalledTimes(1)
    expect(win.close).toHaveBeenCalledTimes(1)
  })

  it("T-ui-winctl-11 [Business rule] the header is a drag region (header + wrapper + spacer), the back button is not, and the title bar stays reachable", async () => {
    // Arrange + Act
    const { header } = await openSettings()
    // Assert
    expect(header).toHaveAttribute("data-tauri-drag-region")
    expect(header.querySelectorAll("[data-tauri-drag-region]").length).toBeGreaterThanOrEqual(2)
    expect(within(header).getByRole("button", { name: /라이브러리로/ })).not.toHaveAttribute("data-tauri-drag-region")
    for (const b of Array.from(header.querySelectorAll("button"))) expect(b).not.toHaveAttribute("data-tauri-drag-region")
  })

  it("T-ui-winctl-12 [Business rule] settings covers only the stage: the rail stays interactive and the status bar stays visible, while the library is hidden from assistive tech", async () => {
    // Arrange + Act
    await openSettings()
    // Assert
    const rail = screen.getByRole("navigation", { name: "주 메뉴" })
    expect(rail.closest("[inert]")).toBeNull()
    expect(rail.closest("[aria-hidden=true]")).toBeNull()
    expect(screen.getByRole("status", { name: "작업 상태" }).closest("[inert]")).toBeNull()
    expect(screen.getByTestId("topbar").closest("[inert]")).not.toBeNull()
  })
})
