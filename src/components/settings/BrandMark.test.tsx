// Categories: Happy (이름·부제·버전) / Boundary (버전 없음 = 개발 빌드, 장식 이미지는 접근성 트리 제외)
import { render, screen, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"
import { BrandMark } from "./BrandMark"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("BrandMark", () => {
  it("T-ui-brand-01 [Happy] renders the name with its trailing period, subtitle and version", () => {
    // Arrange / Act
    render(<BrandMark version="v0.1.0" />)
    // Assert
    expect(screen.getByText("velio.")).toBeInTheDocument()
    expect(screen.getByText("로컬 미디어 자산 관리 · v0.1.0")).toBeInTheDocument()
  })

  it("T-ui-brand-02 [Boundary] the symbol is decorative (no accessible name) so the name is announced once", () => {
    // Arrange / Act
    const { container } = render(<BrandMark version="v0.1.0" />)
    // Assert
    const img = container.querySelector("img")
    expect(img).not.toBeNull()
    expect(img).toHaveAttribute("alt", "")
    expect(img).toHaveAttribute("aria-hidden", "true")
    expect(screen.queryByRole("img")).toBeNull()
  })
})

async function openGeneral() {
  stubLayout()
  const user = userEvent.setup()
  renderApp(createMockApi({ count: 10 }))
  return openSettingsPage(user, "일반")
}

describe("settings > general > brand card", () => {
  it("T-ui-brand-03 [Boundary] without VITE_APP_VERSION the brand card still renders and shows the dev build label", async () => {
    // Arrange
    vi.stubEnv("VITE_APP_VERSION", undefined as unknown as string)
    // Act
    const page = await openGeneral()
    // Assert
    const card = await within(page).findByTestId("brand-mark")
    expect(within(card).getByText("velio.")).toBeInTheDocument()
    expect(card).toHaveTextContent("로컬 미디어 자산 관리 · 개발 빌드")
  })

  it("T-ui-brand-04 [Happy] with a version the card shows v<version> and sits above the version row", async () => {
    // Arrange
    vi.stubEnv("VITE_APP_VERSION", "1.2.3")
    // Act
    const page = await openGeneral()
    // Assert
    const card = await within(page).findByTestId("brand-mark")
    expect(card).toHaveTextContent("로컬 미디어 자산 관리 · v1.2.3")
    const versionLabel = within(page).getByText("버전")
    expect(card.compareDocumentPosition(versionLabel) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
  })
})
