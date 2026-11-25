// Categories: Happy (T-ui-rating-menu-01) / Business rule (T-ui-rating-menu-02..05)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import App from "@/App"
import { qk } from "@/hooks/keys"
import { createMockApi } from "@/lib/api/mock"
import * as ipcMetadata from "@/lib/ipc-metadata"
import { resetAllStores, useSearchStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { FilterPopover } from "./FilterPopover"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const nav = () => screen.findByRole("navigation", { name: "라이브러리 탐색" })
const ratingGroup = async () => within(await nav()).findByRole("region", { name: "별점" })

describe("sidebar rating menu", () => {
  it("T-ui-rating-menu-01 [Happy] shows the group by default with ★5..★1 + 미평가 and counts from rating_counts", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    vi.spyOn(api, "ratingCounts").mockResolvedValue([10, 1, 2, 3, 4, 5])
    // Act
    renderApp(api)
    const group = await ratingGroup()
    // Assert
    await waitFor(() =>
      expect(within(group).getAllByRole("button").map((b) => b.textContent)).toEqual([
        "별점 5점5",
        "별점 4점4",
        "별점 3점3",
        "별점 2점2",
        "별점 1점1",
        "미평가10",
      ])
    )
  })

  it("T-ui-rating-menu-02 [Business rule] clicking a star item selects the exact rating view", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    const group = await ratingGroup()
    // Act
    await user.click(within(group).getByRole("button", { name: /별점 4점/ }))
    // Assert
    expect(useSearchStore.getState().view).toEqual({ kind: "rating", value: 4 })
    expect(within(group).getByRole("button", { name: /별점 4점/ })).toHaveAttribute("aria-current", "true")
  })

  it("T-ui-rating-menu-03 [Business rule] setting off: group absent and rating_counts is never called", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 50 })
    await api.updateSettings({ sidebarRatingMenu: false })
    const counts = vi.spyOn(api, "ratingCounts")
    // Act
    renderApp(api)
    await nav()
    await screen.findAllByRole("option") // 설정·목록 조회가 끝난 뒤
    // Assert
    expect(within(await nav()).queryByRole("region", { name: "별점" })).not.toBeInTheDocument()
    expect(counts).not.toHaveBeenCalled()
  })

  it("T-ui-rating-menu-04 [Business rule] turning the menu off while a rating view is selected falls back to the all view", async () => {
    // Arrange
    stubLayout()
    resetAllStores()
    const api = createMockApi({ count: 50 })
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
    render(<App api={api} queryClient={queryClient} />)
    await ratingGroup()
    act(() => useSearchStore.getState().setView({ kind: "rating", value: 3 }))
    // Act: 설정 화면 저장 경로와 같이 설정 캐시를 갱신
    const settings = await api.getSettings()
    act(() => queryClient.setQueryData(qk.settings, { ...settings, sidebarRatingMenu: false }))
    // Assert
    await waitFor(() => expect(useSearchStore.getState().view).toEqual({ kind: "all" }))
    expect(within(await nav()).queryByRole("region", { name: "별점" })).not.toBeInTheDocument()
  })

  it("T-ui-rating-menu-05 [Business rule] the filter popover rating row works independently of the sidebar menu setting", async () => {
    // Arrange: 사이드바 없이 팝오버만 렌더 — 메뉴 설정과 무관한 경로
    Element.prototype.hasPointerCapture ??= () => false
    Element.prototype.scrollIntoView ??= () => undefined
    resetAllStores()
    const user = userEvent.setup()
    vi.spyOn(ipcMetadata, "listMetadataFields").mockResolvedValue([])
    render(
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <FilterPopover />
      </QueryClientProvider>
    )
    await user.click(screen.getByRole("button", { name: "필터" }))
    // Act
    await user.click(screen.getByRole("combobox", { name: "최소 별점" }))
    await user.click(within(await screen.findByRole("listbox")).getByRole("option", { name: "★2" }))
    // Assert
    expect(useSearchStore.getState().filters.ratingMin).toBe(2)
  })
})
