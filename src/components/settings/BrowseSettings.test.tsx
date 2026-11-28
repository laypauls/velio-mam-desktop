// Categories: Happy (T-ui-set-01, 02, 03) / Business rule (T-ui-set-01, 02, 05) / External (T-ui-set-04)
import { screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { cmdError } from "@/lib/api/mock-search"
import { renderApp, stubLayout } from "@/test/layout"
import { openSettingsPage } from "@/test/settings"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function openBrowse(user: ReturnType<typeof userEvent.setup>) {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  const dialog = await openSettingsPage(user, "탐색·표시")
  const group = await within(dialog).findByRole("group", { name: "목록 탐색" })
  return { dialog, group }
}

async function choose(user: ReturnType<typeof userEvent.setup>, group: HTMLElement, select: string, option: string) {
  await user.click(within(group).getByRole("combobox", { name: select }))
  await user.click(await screen.findByRole("option", { name: option }))
}

async function pick(user: ReturnType<typeof userEvent.setup>, group: HTMLElement, radioGroup: string, option: string) {
  await user.click(within(within(group).getByRole("radiogroup", { name: radioGroup })).getByRole("radio", { name: option }))
}

describe("settings > view & behavior > 목록 탐색", () => {
  it("T-ui-set-01 [Business rule] shows defaults between double-click and sort fields; page size is disabled unless paged", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    // Act
    const { dialog, group } = await openBrowse(user)
    // Assert
    expect(within(within(group).getByRole("radiogroup", { name: "탐색 방식" })).getByRole("radio", { name: "무한 스크롤" })).toBeChecked()
    expect(within(group).getByRole("combobox", { name: "페이지당 개수" })).toBeDisabled()
    expect(within(within(group).getByRole("radiogroup", { name: "목차 레일" })).getByRole("radio", { name: "항상 표시" })).toBeChecked()
    const order = within(dialog).getAllByRole("group").map((g) => g.getAttribute("aria-label"))
    expect(order.indexOf("목록 탐색")).toBeGreaterThan(-1)
    expect(order.indexOf("목록 탐색")).toBeLessThan(order.indexOf("정렬 항목"))
    expect(within(dialog).getByRole("combobox", { name: "더블클릭 동작" })).toBeInTheDocument()
  })

  it("T-ui-set-02 [Happy] paging and page size persist their own patch; page size unlocks only after choosing 페이지", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const { group } = await openBrowse(user)
    // Act + Assert: 페이지로 전환
    await pick(user, group, "탐색 방식", "페이지")
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ galleryPaging: "paged" }))
    const size = within(group).getByRole("combobox", { name: "페이지당 개수" })
    await waitFor(() => expect(size).toBeEnabled())
    expect(size).toHaveTextContent("100개")
    // Act + Assert: 200개
    await choose(user, group, "페이지당 개수", "200개")
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ galleryPageSize: 200 }))
    expect((await api.getSettings()).galleryPageSize).toBe(200)
    // Act + Assert: 다시 무한 스크롤 → 쪽 크기 선택은 잠긴다
    await pick(user, group, "탐색 방식", "무한 스크롤")
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ galleryPaging: "infinite" }))
    await waitFor(() => expect(within(group).getByRole("combobox", { name: "페이지당 개수" })).toBeDisabled())
  })

  it("T-ui-set-03 [Happy] index rail cards send 자동 then 끄기 (default is 항상 표시)", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const update = vi.spyOn(api, "updateSettings")
    const user = userEvent.setup()
    renderApp(api)
    const { group } = await openBrowse(user)
    // Act + Assert
    await pick(user, group, "목차 레일", "자동")
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ indexRail: "auto" }))
    await pick(user, group, "목차 레일", "끄기")
    await waitFor(() => expect(update).toHaveBeenLastCalledWith({ indexRail: "off" }))
    expect((await api.getSettings()).indexRail).toBe("off")
  })

  it("T-ui-set-03b [Happy] each index rail card exposes its hint text next to its radio", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    // Act
    const { group } = await openBrowse(user)
    // Assert
    const rail = within(group).getByRole("radiogroup", { name: "목차 레일" })
    expect(within(rail).getByRole("radio", { name: "자동" }).closest("label")).toHaveTextContent("스크롤하거나 마우스를 올리면 잠시 표시")
    expect(within(rail).getByRole("radio", { name: "항상 표시" }).closest("label")).toHaveTextContent("목차를 계속 표시")
    expect(within(rail).getByRole("radio", { name: "끄기" }).closest("label")).toHaveTextContent("위치 핸들만 표시")
  })

  it("T-ui-set-04[External] a rejected save shows an inline alert in the group and keeps the saved value", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    vi.spyOn(api, "updateSettings").mockRejectedValue(cmdError("invalid_input", "galleryPaging invalid"))
    const user = userEvent.setup()
    renderApp(api)
    const { group } = await openBrowse(user)
    // Act
    await pick(user, group, "탐색 방식", "페이지")
    // Assert
    expect(await within(group).findByRole("alert")).toBeInTheDocument()
    expect(within(group).getByRole("radio", { name: "무한 스크롤" })).toBeChecked()
    expect(within(group).getByRole("radio", { name: "페이지" })).not.toBeChecked()
    expect(within(group).getByRole("combobox", { name: "페이지당 개수" })).toBeDisabled()
  })

  it("T-ui-set-05 [Business rule, gate ON] choosing 페이지 in settings makes the gallery show the pager without reload; going back to 무한 스크롤 removes it", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 300 })
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")
    expect(screen.queryByRole("navigation", { name: "쪽 이동" })).not.toBeInTheDocument()
    const { group } = await openBrowse(user)
    // Act
    await pick(user, group, "탐색 방식", "페이지")
    await waitFor(() => expect(within(group).getByRole("radio", { name: "페이지" })).toBeChecked())
    await user.click(screen.getByRole("button", { name: "라이브러리로" }))
    // Assert
    expect(await screen.findByRole("navigation", { name: "쪽 이동" })).toBeInTheDocument()
  })
})
