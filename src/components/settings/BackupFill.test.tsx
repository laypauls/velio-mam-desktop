// Categories: Business rule (T-ui-backup-fill-01)
// jsdom 은 레이아웃을 계산하지 않으므로 구조 계약만 단언한다 — 실제 높이 채움은 tauri dev(CDP) 실화면에서 확인한다.
import { within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { BackupInfo } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const AT = Date.UTC(2026, 10, 6, 0, 0)
const BACKUPS: BackupInfo[] = Array.from({ length: 6 }, (_, i) => ({
  fileName: `auto-${i}.db`,
  createdAt: AT - i * 3_600_000,
  sizeBytes: 1_000,
  kind: "auto",
}))

describe("backup list grows to content height", () => {
  // 병합 후 백업 목록 아래에 다른 섹션이 이어지므로 "남은 높이 채움 + 내부 스크롤" 이 아니라 페이지(main) 스크롤을 쓴다.
  const noInnerScroll = (el: HTMLElement | null) => {
    expect(el).not.toBeNull()
    expect(el?.className).not.toMatch(/(^|\s)(max-h-|overflow-y-|overflow-auto|min-h-0|basis-0|flex-1)/)
  }

  it("T-ui-backup-fill-01 [Business rule] list is not an inner scroll container; it grows with content and the page scrolls", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 300, backups: BACKUPS }))
    const settings = await openSettingsPage(user, "데이터·백업")
    // Act
    const list = await within(settings).findByRole("list", { name: "백업 목록" })
    // Assert
    noInnerScroll(list)
    noInnerScroll(list.parentElement)
    expect(list.className).not.toContain("min-h-40")
    expect(list.parentElement?.className).not.toContain("min-h-40")
  })

  it("T-ui-backup-fill-02 [Boundary] many backups still render every row without an inner scroll container", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const many: BackupInfo[] = Array.from({ length: 40 }, (_, i) => ({
      fileName: `auto-${i}.db`,
      createdAt: AT - i * 3_600_000,
      sizeBytes: 1_000,
      kind: "auto",
    }))
    renderApp(createMockApi({ count: 300, backups: many }))
    const settings = await openSettingsPage(user, "데이터·백업")
    // Act
    const list = await within(settings).findByRole("list", { name: "백업 목록" })
    // Assert
    expect(within(list).getAllByRole("listitem")).toHaveLength(40)
    noInnerScroll(list)
  })
})
