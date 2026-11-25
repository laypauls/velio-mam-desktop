// Categories: Happy (T-ui-sbar-01, 04) / Boundary (T-ui-sbar-02) / Business rule (T-ui-sbar-03, 04) / Regression (T-ui-sbar-05)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { PlayQueueItem } from "@/lib/ipc-contract"
import { usePlaybackStore, useSearchStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const nav = () => screen.getByRole("navigation", { name: "라이브러리 탐색" })

describe("sidebar tabs", () => {
  it("T-ui-sbar-01 [Happy] switching to the collections tab shows the collection list and hides the library groups", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    await within(await screen.findByRole("navigation", { name: "라이브러리 탐색" })).findByRole("tab", { name: /^라이브러리/ })
    expect(within(nav()).getByRole("region", { name: "라이브러리" })).toBeInTheDocument()
    // Act
    await user.click(within(nav()).getByRole("tab", { name: /^컬렉션/ }))
    // Assert
    expect(await within(nav()).findByRole("button", { name: /^포트폴리오/ })).toBeInTheDocument()
    expect(within(nav()).queryByRole("region", { name: "라이브러리" })).not.toBeInTheDocument()
    expect(within(nav()).queryByRole("button", { name: /^전체 미디어/ })).not.toBeInTheDocument()
    expect(within(nav()).getByRole("tab", { name: /^컬렉션/ })).toHaveAttribute("aria-selected", "true")
  })

  it("T-ui-sbar-02 [Boundary] the collections tab name is just the label, with no count, whether there are collections or none", async () => {
    // Arrange: 기본 모의 데이터는 컬렉션 3개
    stubLayout()
    const api = createMockApi({ count: 50 })
    expect(await api.listCollections()).toHaveLength(3)
    const first = renderApp(api)
    // Act + Assert: 3개여도 숫자 없이 이름만
    await waitFor(() => expect(within(nav()).getByRole("tab", { name: "컬렉션" })).toBeInTheDocument())
    first.unmount()
    // Arrange: 모두 삭제
    for (const c of await api.listCollections()) await api.deleteCollection(c.collectionId)
    renderApp(api)
    // Assert: 없을 때도 이름만
    await waitFor(() => expect(within(nav()).getByRole("tab", { name: "컬렉션" })).toBeInTheDocument())
  })

  it("T-ui-sbar-03 [Business rule] the library tab has no collections group; the collections tab has it", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    await screen.findByRole("tab", { name: /^컬렉션/ })
    // Assert: 라이브러리 탭
    expect(within(nav()).queryByRole("region", { name: "컬렉션" })).not.toBeInTheDocument()
    expect(within(nav()).queryByRole("button", { name: "컬렉션 추가" })).not.toBeInTheDocument()
    // Act
    await user.click(within(nav()).getByRole("tab", { name: /^컬렉션/ }))
    // Assert: 컬렉션 탭
    expect(within(nav()).getByRole("region", { name: "컬렉션" })).toBeInTheDocument()
    expect(within(nav()).getByRole("button", { name: "컬렉션 추가" })).toBeInTheDocument()
  })

  it("T-ui-sbar-04 [Business rule] the tab is kept while navigating, and the playing collection shows the play marker", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 50 })
    renderApp(api)
    await user.click(await screen.findByRole("tab", { name: /^컬렉션/ }))
    const row = await within(nav()).findByRole("button", { name: /^부산 여행/ })
    // Act: 컬렉션 보기로 이동해도 탭은 그대로
    await user.click(row)
    expect(useSearchStore.getState().view).toEqual({ kind: "collection", collectionId: 2 })
    expect(useViewStore.getState().sidebarTab).toBe("collections")
    expect(within(row).queryByRole("img", { name: "재생 중" })).not.toBeInTheDocument()
    // 재생 시작 → 해당 컬렉션 항목에 표시
    const item: PlayQueueItem = { assetId: 1, mediaType: "image", availability: "online", fileName: "a.jpg", title: null, durationMs: null }
    vi.spyOn(api, "collectionPlayQueue").mockResolvedValue([item])
    act(() => usePlaybackStore.getState().start(2, [item]))
    // Assert
    expect(await within(row).findByRole("img", { name: "재생 중" })).toBeInTheDocument()
    expect(within(nav()).queryAllByRole("img", { name: "재생 중" })).toHaveLength(1)
  })

  it("T-ui-sbar-05 [Regression] the library tab no longer renders a saved-search group (feature removed)", async () => {
    // Arrange
    stubLayout()
    renderApp(createMockApi({ count: 50 }))
    // Act
    await within(await screen.findByRole("navigation", { name: "라이브러리 탐색" })).findByRole("region", { name: "라이브러리" })
    // Assert
    expect(within(nav()).queryByRole("region", { name: "저장한 검색" })).not.toBeInTheDocument()
    expect(within(nav()).queryByText("저장한 검색이 없습니다.")).not.toBeInTheDocument()
    expect(within(nav()).getByRole("region", { name: "태그" })).toBeInTheDocument()
  })
})
