// Categories: Happy + Business rule (T-ui-view-01~03) / Boundary (T-ui-view-03)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function openGeneral(user: ReturnType<typeof userEvent.setup>) {
  const dialog = await openSettingsPage(user, "탐색·표시")
  await within(dialog).findByRole("group", { name: "정렬 항목" })
  return dialog
}

describe("settings > general > view & behavior", () => {
  it("T-ui-view-01 [Happy] title edit switch saves ON then OFF (stored titles are not touched)", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openGeneral(user)
    const sw = within(dialog).getByRole("switch", { name: "콘텐츠 제목 편집" })
    expect(sw).not.toBeChecked() // 기본값 꺼짐
    // Act + Assert: ON
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ titleEditEnabled: true }))
    await waitFor(() => expect(sw).toBeChecked())
    // Act + Assert: OFF
    await user.click(sw)
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ titleEditEnabled: false }))
    expect((await api.getSettings()).titleEditEnabled).toBe(false)
  })

  it("T-ui-view-02 [Happy] double-click action select saves openSystem and back to preview", async () => {
    // Arrange
    stubLayout()
    Element.prototype.hasPointerCapture ??= () => false
    Element.prototype.scrollIntoView ??= () => undefined
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openGeneral(user)
    const select = within(dialog).getByRole("combobox", { name: "더블클릭 동작" })
    expect(select).toHaveTextContent("미리보기")
    // Act + Assert
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "기본 프로그램으로 열기" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ doubleClickAction: "openSystem" }))
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "미리보기" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ doubleClickAction: "preview" }))
  })

  it("T-ui-view-03 [Boundary] relevance is always on; sort fields send the whole object; the last enabled field cannot be turned off", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openGeneral(user)
    const group = within(dialog).getByRole("group", { name: "정렬 항목" })
    const relevance = within(group).getByRole("checkbox", { name: "관련도" })
    expect(relevance).toBeChecked()
    expect(relevance).toBeDisabled()
    expect(within(group).getByText("항상")).toBeInTheDocument()
    // Act: 크기 끄기 → 전체 객체 전송
    await user.click(within(group).getByRole("checkbox", { name: "크기" }))
    await waitFor(() =>
      expect(update).toHaveBeenLastCalledWith({
        sortFieldsEnabled: { capturedAt: true, modifiedAt: true, name: true, size: false, duration: true, rating: true },
      })
    )
    // Act: 하나만 남을 때까지 끄기
    for (const name of ["촬영일", "수정일", "제목", "별점"]) {
      await user.click(within(group).getByRole("checkbox", { name }))
    }
    // Assert: 마지막 하나(길이)는 끌 수 없다
    const last = within(group).getByRole("checkbox", { name: "길이" })
    await waitFor(() => expect(last).toBeDisabled())
    expect(last).toBeChecked()
    await user.click(last)
    expect((await api.getSettings()).sortFieldsEnabled).toEqual({ capturedAt: false, modifiedAt: false, name: false, size: false, duration: true, rating: false })
    // 다시 하나를 켜면 마지막 항목도 끌 수 있다
    await user.click(within(group).getByRole("checkbox", { name: "크기" }))
    await waitFor(() => expect(last).toBeEnabled())
  })

  it("T-ui-view-05 [Happy] sort field list mirrors the toolbar dropdown order; label click toggles; hidden items show 숨김", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openGeneral(user)
    const group = within(dialog).getByRole("group", { name: "정렬 항목" })
    // Assert: 드롭다운과 같은 순서, 처음에는 숨김 표시 없음
    expect(within(group).getAllByRole("checkbox").map((c) => c.getAttribute("aria-label"))).toEqual([
      "관련도", "촬영일", "수정일", "제목", "크기", "길이", "별점",
    ])
    expect(within(group).queryByText("숨김")).not.toBeInTheDocument()
    // Act: 행의 글자를 눌러 수정일 끄기
    await user.click(within(group).getByText("수정일"))
    // Assert
    await waitFor(() =>
      expect(update).toHaveBeenLastCalledWith({
        sortFieldsEnabled: { capturedAt: true, modifiedAt: false, name: true, size: true, duration: true, rating: true },
      })
    )
    expect(await within(group).findByText("숨김")).toBeInTheDocument()
    expect(within(group).getByRole("checkbox", { name: "수정일" })).not.toBeChecked()
  })
})

describe("settings > view & behavior > captured date fallback", () => {
  it("T-ui-view-04 [Happy] select defaults to 수정일 and saves 'none' (빈값) then 'modified'", async () => {
    // Arrange
    stubLayout()
    Element.prototype.hasPointerCapture ??= () => false
    Element.prototype.scrollIntoView ??= () => undefined
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const dialog = await openGeneral(user)
    const select = within(dialog).getByRole("combobox", { name: "촬영일이 없을 때" })
    expect(select).toHaveTextContent("수정일")
    // Act + Assert
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "빈값" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ capturedAtFallback: "none" }))
    await waitFor(() => expect(select).toHaveTextContent("빈값"))
    await user.click(select)
    await user.click(await screen.findByRole("option", { name: "수정일" }))
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ capturedAtFallback: "modified" }))
    expect((await api.getSettings()).capturedAtFallback).toBe("modified")
  })
})
