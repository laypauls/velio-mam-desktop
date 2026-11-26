// Categories: Happy (T-ui-boot-04 자리표시가 있고 마운트로 교체됨) / Boundary (스크립트 로드 전 빈 #root 가 아님 — 텍스트 없는 중앙 심볼 + 접근 이름, reduced-motion 정지)
import { render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import App from "@/App"
import { createMockApi } from "@/lib/api/mock"
import { resetAllStores } from "@/stores"
import { stubLayout } from "@/test/layout"
import indexHtml from "../index.html?raw"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

// 체크아웃은 LF 지만 도구가 새로 만든 파일은 CRLF 일 수 있다
const html = indexHtml.replace(/\r\n/g, "\n")

describe("first-window placeholder", () => {
  it("T-ui-boot-04a [Boundary] index.html ships a status placeholder inside #root so the window is never blank before the script loads", () => {
    // Arrange / Act
    const doc = new DOMParser().parseFromString(html, "text/html")
    const root = doc.getElementById("root")
    // Assert
    expect(root).not.toBeNull()
    const status = root?.querySelector("[role=status]")
    expect(status).not.toBeNull()
    expect(status?.getAttribute("aria-label")?.trim()).toBeTruthy()
    expect(status?.querySelector("svg")).not.toBeNull()
    expect(status?.textContent?.trim()).toBe("")
    expect(html.indexOf('id="root"')).toBeLessThan(html.indexOf("/src/main.tsx"))
  })

  it("T-ui-boot-04c [Boundary] splash style honours prefers-reduced-motion and stays scoped to the splash", () => {
    // Arrange / Act
    const css = new DOMParser().parseFromString(html, "text/html").querySelector("head style")?.textContent ?? ""
    const reduced = css.slice(css.indexOf("@media (prefers-reduced-motion: reduce)"))
    // Assert
    expect(css).toContain("@media (prefers-reduced-motion: reduce)")
    expect(reduced).toMatch(/\[data-boot-splash\][^{]*\{[^}]*animation:\s*none/)
  })

  it("T-ui-boot-04b [Happy] mounting the app replaces the placeholder", async () => {
    // Arrange
    stubLayout()
    resetAllStores()
    const doc = new DOMParser().parseFromString(html, "text/html")
    const container = document.body.appendChild(doc.getElementById("root") as HTMLElement)
    // Act
    render(<App api={createMockApi({ count: 10 })} />, { container })
    // Assert
    await screen.findAllByRole("option")
    expect(container.querySelector("[data-boot-splash]")).toBeNull()
  })
})
