// Categories: Happy (T-ui-sbar-01, 04, 06) / Boundary (T-ui-sbar-02 collections tab name, 07 tag cap, 08 empty/no-match) / Business rule (T-ui-sbar-03 one section at a time, 04 playing marker, 09 tag selection rules) / Regression (T-ui-sbar-05)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import { toast } from "sonner"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { PlayQueueItem } from "@/lib/ipc-contract"
import { useActionsStore, usePlaybackStore, useSearchStore, useViewStore } from "@/stores"
import { openSidebarSection, renderApp, stubLayout } from "@/test/layout"
import { MAX_TAG_CHIPS } from "./useTagAutocomplete"
import { nextTagIds } from "./SidebarTags"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const nav = () => screen.getByRole("navigation", { name: "라이브러리 탐색" })

describe("sidebar section panels", () => {
  it("T-ui-sbar-01 [Happy] the default section is the library; the collections section shows the list and hides the library group", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    renderApp(createMockApi({ count: 50 }))
    expect(await within(await screen.findByRole("navigation", { name: "라이브러리 탐색" })).findByRole("region", { name: "라이브러리" })).toBeInTheDocument()
    expect(within(nav()).getByRole("button", { name: /^전체 미디어/ })).toBeInTheDocument()
    // Act
    await user.click(screen.getByRole("button", { name: "컬렉션" }))
    // Assert
    expect(await within(nav()).findByRole("button", { name: /^포트폴리오/ })).toBeInTheDocument()
    expect(within(nav()).queryByRole("region", { name: "라이브러리" })).not.toBeInTheDocument()
    expect(within(nav()).queryByRole("button", { name: /^전체 미디어/ })).not.toBeInTheDocument()
  })

  it("T-ui-sbar-02 [Boundary] a section shows only itself — the library panel has no tags/folders/collections/rating groups", async () => {
    // Arrange + Act
    stubLayout()
    renderApp(createMockApi({ count: 50 }))
    await within(await screen.findByRole("navigation", { name: "라이브러리 탐색" })).findByRole("region", { name: "라이브러리" })
    // Assert
    for (const name of ["태그", "관리 폴더", "컬렉션", "별점"]) {
      expect(within(nav()).queryByRole("region", { name })).not.toBeInTheDocument()
    }
  })

  it("T-ui-sbar-03 [Business rule] the collections section has its add button; the folders section its add-folder button; tags a manage button that opens the tag manager", async () => {
    // Arrange
    stubLayout()
    renderApp(createMockApi({ count: 50 }))
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    // Act + Assert: 컬렉션
    openSidebarSection("collections")
    expect(within(nav()).getByRole("button", { name: "컬렉션 추가" })).toBeInTheDocument()
    // 관리 폴더
    openSidebarSection("folders")
    expect(within(nav()).getByRole("button", { name: "관리 폴더 추가" })).toBeInTheDocument()
    expect(within(nav()).queryByRole("button", { name: "컬렉션 추가" })).not.toBeInTheDocument()
    // 태그
    openSidebarSection("tags")
    expect(useActionsStore.getState().tagManagerOpen).toBe(false)
    await userEvent.setup().click(within(nav()).getByRole("button", { name: "태그 관리" }))
    expect(useActionsStore.getState().tagManagerOpen).toBe(true)
  })

  it("T-ui-sbar-04 [Business rule] the section is kept while navigating, and the playing collection shows the play marker", async () => {
    // Arrange: 폭이 넉넉해야 재생으로 보기 영역이 열려도 패널이 자동으로 접히지 않는다
    stubLayout({ width: 1600 })
    const user = userEvent.setup()
    const api = createMockApi({ count: 50 })
    renderApp(api)
    await user.click(await screen.findByRole("button", { name: "컬렉션" }))
    const row = await within(nav()).findByRole("button", { name: /^부산 여행/ })
    // Act: 컬렉션 보기로 이동해도 섹션은 그대로
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

  it("T-ui-sbar-05 [Regression] the library section no longer renders a saved-search group (feature removed)", async () => {
    // Arrange
    stubLayout()
    renderApp(createMockApi({ count: 50 }))
    // Act
    await within(await screen.findByRole("navigation", { name: "라이브러리 탐색" })).findByRole("region", { name: "라이브러리" })
    // Assert
    expect(within(nav()).queryByRole("region", { name: "저장한 검색" })).not.toBeInTheDocument()
    expect(within(nav()).queryByText("저장한 검색이 없습니다.")).not.toBeInTheDocument()
  })
})

describe("sidebar tags section", () => {
  async function setupTags() {
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 50 })
    const tags = await api.listTags()
    renderApp(api)
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    openSidebarSection("tags")
    await within(nav()).findByRole("button", { name: new RegExp(`^${tags[0].name}`) })
    return { user, tags }
  }

  it("T-ui-sbar-06 [Happy] lists tags with their counts and clicking one replaces filters.tagIds; clicking the active tag clears it", async () => {
    // Arrange
    const { user, tags } = await setupTags()
    const first = within(nav()).getByRole("button", { name: new RegExp(`^${tags[0].name}`) })
    // Act
    await user.click(first)
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([tags[0].tagId])
    expect(first).toHaveAttribute("aria-current", "true")
    // Act: 다른 태그 — 교체
    await user.click(within(nav()).getByRole("button", { name: new RegExp(`^${tags[1].name}`) }))
    expect(useSearchStore.getState().filters.tagIds).toEqual([tags[1].tagId])
    // Act: 활성 태그 재클릭 — 해제
    await user.click(within(nav()).getByRole("button", { name: new RegExp(`^${tags[1].name}`) }))
    expect(useSearchStore.getState().filters.tagIds).toEqual([])
  })

  it("T-ui-sbar-09 [Business rule] Ctrl+click adds a tag (AND) and clicking one of several active tags removes only that one", async () => {
    // Arrange
    const { user, tags } = await setupTags()
    const byName = (i: number) => within(nav()).getByRole("button", { name: new RegExp(`^${tags[i].name}`) })
    // Act
    await user.click(byName(0))
    await user.keyboard("{Control>}")
    await user.click(byName(1))
    await user.keyboard("{/Control}")
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([tags[0].tagId, tags[1].tagId])
    // Act: 활성 태그 하나만 해제
    await user.click(byName(0))
    expect(useSearchStore.getState().filters.tagIds).toEqual([tags[1].tagId])
  })

  it("T-ui-sbar-07 [Boundary] nextTagIds respects the MAX_TAG_CHIPS cap for additive picks (limit) but a plain pick or removal still works at the cap", () => {
    // Arrange
    const full = Array.from({ length: MAX_TAG_CHIPS }, (_, i) => i + 1)
    // Act + Assert
    expect(nextTagIds(full, 999, true)).toBe("limit")
    expect(nextTagIds(full, 999, false)).toEqual([999])
    expect(nextTagIds(full, 3, true)).toEqual(full.filter((x) => x !== 3))
    expect(nextTagIds([], 5, true)).toEqual([5])
  })

  it("T-ui-sbar-07b [Boundary] Ctrl+click at the cap leaves tagIds unchanged and tells the user", async () => {
    // Arrange
    stubLayout()
    const user = userEvent.setup()
    const api = createMockApi({ count: 5 })
    const many = Array.from({ length: MAX_TAG_CHIPS + 1 }, (_, i) => ({ tagId: i + 1, name: `tag-${i + 1}`, assetCount: 1 }))
    vi.spyOn(api, "listTags").mockResolvedValue(many)
    renderApp(api)
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    openSidebarSection("tags")
    // 라벨과 개수가 붙어 읽혀 "tag-5"+"1" 과 구분되지 않으므로 라벨 텍스트로 찾는다
    const last = (await within(nav()).findByText("tag-51")).closest("button") as HTMLButtonElement
    const info = vi.spyOn(toast, "info").mockReturnValue("t")
    const full = many.slice(0, MAX_TAG_CHIPS).map((x) => x.tagId)
    act(() => useSearchStore.getState().patchFilters({ tagIds: full }))
    // Act
    await user.keyboard("{Control>}")
    await user.click(last)
    await user.keyboard("{/Control}")
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual(full)
    expect(info).toHaveBeenCalledWith(`태그는 최대 ${MAX_TAG_CHIPS}개까지 고를 수 있습니다`)
  })

  it("T-ui-sbar-08 [Boundary] the search box narrows the list; no match and no tags show distinct messages", async () => {
    // Arrange
    const { user, tags } = await setupTags()
    const box = within(nav()).getByRole("searchbox", { name: "태그 검색" })
    // Act: 일치
    await user.type(box, tags[0].name.toUpperCase())
    // Assert
    expect(within(nav()).getByRole("button", { name: new RegExp(`^${tags[0].name}`) })).toBeInTheDocument()
    // Act: 불일치
    await user.clear(box)
    await user.type(box, "zzzz-없는-태그")
    // Assert
    expect(within(nav()).getByText("일치하는 태그가 없습니다.")).toBeInTheDocument()
    expect(within(nav()).queryAllByRole("button", { name: /\d$/ })).toHaveLength(0)
  })

  it("T-ui-sbar-10 [External] a failing tag list shows an alert instead of an empty list; no tags at all shows the empty message", async () => {
    // Arrange: 실패
    stubLayout()
    const failing = createMockApi({ count: 5 })
    vi.spyOn(failing, "listTags").mockRejectedValue({ code: "io", message: "disk" })
    const first = renderApp(failing)
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    openSidebarSection("tags")
    // Assert
    expect(await within(nav()).findByRole("alert")).toHaveTextContent("태그 목록을 불러오지 못했습니다.")
    first.unmount()
    // Arrange: 태그 없음
    const empty = createMockApi({ count: 5 })
    vi.spyOn(empty, "listTags").mockResolvedValue([])
    renderApp(empty)
    await screen.findByRole("navigation", { name: "라이브러리 탐색" })
    openSidebarSection("tags")
    // Assert
    await waitFor(() => expect(within(nav()).getByText("등록된 태그가 없습니다.")).toBeInTheDocument())
  })
})
