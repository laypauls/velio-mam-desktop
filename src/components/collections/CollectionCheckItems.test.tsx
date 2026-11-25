// Categories: Happy (T-ui-memb-01, 03, 05) / Boundary (T-ui-memb-02, 04) / External (T-ui-memb-06)
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { DEFAULT_FILTERS, DEFAULT_SORT, useSelectionStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/**
 * 첫 두 자산(a, b)의 소속을 고정한다: 포트폴리오(1) = a·b 둘 다, 부산 여행(2) = a 만, 클라이언트 A(3) = 없음.
 * 이름은 모의 데이터의 기본 컬렉션 3개.
 */
async function arrange(api: UiApi): Promise<[number, number]> {
  const page = await api.searchAssets({ query: "", filters: DEFAULT_FILTERS, sort: DEFAULT_SORT, limit: 2, cursor: null })
  const [a, b] = page.items.map((i) => i.assetId)
  for (const id of [1, 2, 3]) await api.removeFromCollection(id, [a, b])
  await api.addToCollection(1, [a, b])
  await api.addToCollection(2, [a])
  return [a, b]
}

/** 항목을 찾고, 소속 조회가 끝나(aria-busy 해제) 눌러도 되는 상태가 될 때까지 기다린다 */
async function check(name: RegExp | string) {
  const item = await screen.findByRole("menuitemcheckbox", { name })
  await waitFor(() => expect(item).not.toHaveAttribute("aria-busy"))
  return item
}

async function openContextCollections(user: ReturnType<typeof userEvent.setup>, a: number, b: number) {
  const first = await screen.findByRole("option", { name: (n) => n.length > 0 && document.querySelector(`[data-asset-id="${a}"]`)?.getAttribute("aria-label") === n })
  act(() => useSelectionStore.getState().selectAll([a, b]))
  fireEvent.contextMenu(first)
  await user.click(await screen.findByRole("menuitem", { name: "컬렉션에 추가" }))
}

describe("collection membership checks", () => {
  it("T-ui-memb-01 [Happy] right-click menu: all = checked, some = mixed, none = unchecked, with the selection size as header", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const [a, b] = await arrange(api)
    const user = userEvent.setup()
    renderApp(api)
    // Act
    await openContextCollections(user, a, b)
    // Assert
    expect(await check(/^포트폴리오/)).toHaveAttribute("aria-checked", "true")
    expect(await check(/^부산 여행/)).toHaveAttribute("aria-checked", "mixed")
    expect(await check(/^클라이언트 A/)).toHaveAttribute("aria-checked", "false")
    expect(screen.getByText("2개 항목")).toBeInTheDocument()
    expect(screen.getByRole("menuitem", { name: "새 컬렉션" })).toBeInTheDocument()
  })

  it("T-ui-memb-02 [Boundary] while the membership query is pending the checks stay blank, busy and not clickable", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const [a, b] = await arrange(api)
    vi.spyOn(api, "collectionMembership").mockReturnValue(new Promise(() => undefined))
    const add = vi.spyOn(api, "addToCollection")
    const user = userEvent.setup()
    renderApp(api)
    // Act
    await openContextCollections(user, a, b)
    const item = await screen.findByRole("menuitemcheckbox", { name: /^포트폴리오/ })
    // Assert
    expect(item).toHaveAttribute("aria-checked", "false")
    expect(item).toHaveAttribute("aria-busy", "true")
    expect(item).toHaveAttribute("aria-disabled", "true")
    fireEvent.click(item)
    expect(add).not.toHaveBeenCalled()
  })

  it("T-ui-memb-03 [Happy] toggling adds when mixed and removes when checked, keeps the menu open and updates the check", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const [a, b] = await arrange(api)
    const add = vi.spyOn(api, "addToCollection")
    const remove = vi.spyOn(api, "removeFromCollection")
    const user = userEvent.setup()
    renderApp(api)
    await openContextCollections(user, a, b)
    // Act 1: 일부(－) → 추가 (서브메뉴 항목은 pointer 이동 시 jsdom 이 서브메뉴를 닫으므로 click 이벤트로 누른다)
    fireEvent.click(await check(/^부산 여행/))
    // Assert 1
    await waitFor(() => expect(add).toHaveBeenCalledWith(2, [a, b]))
    await waitFor(() => expect(screen.getByRole("menuitemcheckbox", { name: /^부산 여행/ })).toHaveAttribute("aria-checked", "true"))
    // Act 2: 모두(✓) → 제거 — 메뉴는 그대로 열려 있다
    fireEvent.click(screen.getByRole("menuitemcheckbox", { name: /^부산 여행/ }))
    // Assert 2
    await waitFor(() => expect(remove).toHaveBeenCalledWith(2, [a, b]))
    await waitFor(() => expect(screen.getByRole("menuitemcheckbox", { name: /^부산 여행/ })).toHaveAttribute("aria-checked", "false"))
    expect(screen.getByRole("menuitemcheckbox", { name: /^포트폴리오/ })).toBeInTheDocument()
  })

  it("T-ui-memb-04 [Boundary] a selection over 10,000 disables every collection and does not query membership", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const membership = vi.spyOn(api, "collectionMembership")
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")
    act(() => useSelectionStore.getState().selectAll(Array.from({ length: 10_001 }, (_, i) => i + 1)))
    const bar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    // Act
    await user.click(within(bar).getByRole("button", { name: "컬렉션" }))
    // Assert
    expect(await screen.findByRole("menuitemcheckbox", { name: /^포트폴리오/ })).toHaveAttribute("aria-disabled", "true")
    expect(screen.getByText("한 번에 최대 10,000개까지만 지정할 수 있습니다.")).toBeInTheDocument()
    expect(membership).not.toHaveBeenCalled()
  })

  it("T-ui-memb-05 [Happy] the selection bar's 컬렉션 menu shows the same checks and toggles with the menu kept open", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const [a, b] = await arrange(api)
    const add = vi.spyOn(api, "addToCollection")
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")
    act(() => useSelectionStore.getState().selectAll([a, b]))
    const bar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    // Act
    await user.click(within(bar).getByRole("button", { name: "컬렉션" }))
    // Assert: 상태
    expect(await check(/^포트폴리오/)).toHaveAttribute("aria-checked", "true")
    expect(await check(/^부산 여행/)).toHaveAttribute("aria-checked", "mixed")
    expect(await check(/^클라이언트 A/)).toHaveAttribute("aria-checked", "false")
    // Act: 빈칸 → 추가
    await user.click(screen.getByRole("menuitemcheckbox", { name: /^클라이언트 A/ }))
    // Assert
    await waitFor(() => expect(add).toHaveBeenCalledWith(3, [a, b]))
    await waitFor(() => expect(screen.getByRole("menuitemcheckbox", { name: /^클라이언트 A/ })).toHaveAttribute("aria-checked", "true"))
  })

  it("T-ui-memb-06 [External] a failed add reverts the check to the server state", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    const [a, b] = await arrange(api)
    const add = vi.spyOn(api, "addToCollection").mockRejectedValue({ code: "internal", message: "boom" })
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")
    act(() => useSelectionStore.getState().selectAll([a, b]))
    const bar = await screen.findByRole("toolbar", { name: "선택 항목 일괄 작업" })
    await user.click(within(bar).getByRole("button", { name: "컬렉션" }))
    // Act
    await user.click(await check(/^클라이언트 A/))
    // Assert: 낙관적으로 바뀌었다가 실패 후 서버 상태(빈칸)로 되돌아온다
    await waitFor(() => expect(add).toHaveBeenCalled())
    await waitFor(() => expect(screen.getByRole("menuitemcheckbox", { name: /^클라이언트 A/ })).toHaveAttribute("aria-checked", "false"))
  })
})
