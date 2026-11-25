// Categories: Business rule (T-ui-sortctl-01, T-ui-sortctl-02) / Boundary (T-ui-sortctl-02)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useSearchStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const ALL_OFF = { capturedAt: false, modifiedAt: false, name: false, size: false, duration: false, rating: false }

async function openSort(user: ReturnType<typeof userEvent.setup>) {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  await user.click(await screen.findByRole("combobox", { name: "정렬 기준" }))
  return screen.findByRole("listbox")
}

describe("SortControl with enabled sort fields", () => {
  it("T-ui-sortctl-01 [Business rule] shows relevance plus only the enabled fields, with 'name' labelled 제목", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    await api.updateSettings({ sortFieldsEnabled: { ...ALL_OFF, capturedAt: true, name: true } })
    const user = userEvent.setup()
    renderApp(api)
    // Act
    const list = await openSort(user)
    // Assert
    const names = within(list).getAllByRole("option").map((o) => o.textContent)
    expect(names).toEqual(["관련도", "촬영일", "제목"])
  })

  it("T-ui-sortctl-02 [Boundary] current sort that becomes disabled switches to the first enabled field", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    await api.updateSettings({ sortFieldsEnabled: { ...ALL_OFF, modifiedAt: true, size: true } })
    renderApp(api)
    await screen.findAllByRole("option")
    // Act 1: 꺼진 항목(제목)을 현재 정렬로
    act(() => useSearchStore.getState().setSort({ field: "name", direction: "asc" }))
    // Assert 1: 켜진 첫 항목으로 전환, 방향은 유지
    await waitFor(() => expect(useSearchStore.getState().sort).toEqual({ field: "modifiedAt", direction: "asc" }))
  })

  it("T-ui-sortctl-03 [Boundary] with no toggleable field enabled the sort falls back to relevance", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    await api.updateSettings({ sortFieldsEnabled: ALL_OFF })
    // Act (기본 정렬은 촬영일)
    renderApp(api)
    // Assert
    await waitFor(() => expect(useSearchStore.getState().sort.field).toBe("relevance"))
  })
})
