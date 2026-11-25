// Categories: Happy (select, chips) / Boundary (empty input, 50 cap) / Validation (IME, no match) / Business rule (no recent record, store sync, prune) / External (listTags failure)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { Tag } from "@/lib/ipc-contract"
import { DEFAULT_FILTERS, resetAllStores, useSearchStore } from "@/stores"
import { SearchBox } from "./SearchBox"

const TAGS: Tag[] = [
  { tagId: 1, name: "바다", assetCount: 400 },
  { tagId: 2, name: "여름", assetCount: 128 },
  { tagId: 3, name: "여행", assetCount: 57 },
  { tagId: 4, name: "가족여행", assetCount: 12 },
  { tagId: 5, name: "빈태그", assetCount: 0 },
]

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] })
  resetAllStores()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

const advance = (ms: number) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)))

async function setup(listTags: () => Promise<Tag[]> = async () => TAGS) {
  const api = createMockApi({ count: 10 })
  vi.spyOn(api, "listTags").mockImplementation(listTags)
  const record = vi.spyOn(api, "recordRecentSearch")
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <SearchBox />
      </QueryClientProvider>
    </ApiProvider>
  )
  const box = screen.getByRole("combobox", { name: "검색" })
  await advance(300) // 태그 목록 로드 + 초기 디바운스 소진
  return { box, record }
}

async function typeToken(box: HTMLElement, value: string) {
  fireEvent.focus(box)
  fireEvent.change(box, { target: { value } })
  await advance(0)
}

const optionNames = () => screen.getAllByRole("option").map((o) => o.textContent)
const pressKey = (box: HTMLElement, key: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(box, { key, ...init })

describe("SearchBox # tag autocomplete", () => {
  it("T-ui-searchtag-01 [Happy] typing # opens a combobox popup with the most used tags first and zero-count tags hidden", async () => {
    // Arrange
    const { box } = await setup()
    expect(box).toHaveAttribute("aria-expanded", "false")
    // Act
    await typeToken(box, "#")
    // Assert
    expect(box).toHaveAttribute("aria-expanded", "true")
    expect(box).toHaveAttribute("aria-controls", screen.getByRole("listbox").id)
    expect(optionNames()).toEqual(["#바다400", "#여름128", "#여행57", "#가족여행12"])
    expect(screen.getAllByRole("option")[0]).toHaveAttribute("aria-selected", "true")
  })

  it("T-ui-searchtag-02 [Happy] a query filters prefix matches before substring matches", async () => {
    // Arrange
    const { box } = await setup()
    // Act
    await typeToken(box, "#여")
    // Assert
    expect(optionNames()).toEqual(["#여름128", "#여행57", "#가족여행12"])
  })

  it("T-ui-searchtag-03 [Happy] ArrowDown/ArrowUp move the highlight, wrap around and update aria-activedescendant", async () => {
    // Arrange
    const { box } = await setup()
    await typeToken(box, "#여")
    const opts = screen.getAllByRole("option")
    expect(box).toHaveAttribute("aria-activedescendant", opts[0].id)
    // Act + Assert
    pressKey(box, "ArrowDown")
    expect(opts[1]).toHaveAttribute("aria-selected", "true")
    expect(box).toHaveAttribute("aria-activedescendant", opts[1].id)
    pressKey(box, "ArrowUp")
    pressKey(box, "ArrowUp") // 처음에서 위로 → 끝
    expect(opts[2]).toHaveAttribute("aria-selected", "true")
    pressKey(box, "ArrowDown") // 끝에서 아래로 → 처음
    expect(opts[0]).toHaveAttribute("aria-selected", "true")
  })

  it("T-ui-searchtag-04 [Business rule] Enter picks the highlighted tag as a chip filter, removes the token, and neither commits nor records a search", async () => {
    // Arrange
    const { box, record } = await setup()
    await typeToken(box, "노을 #여")
    pressKey(box, "ArrowDown")
    // Act
    pressKey(box, "Enter")
    await advance(0)
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([3])
    expect(box).toHaveValue("노을")
    expect(screen.getByRole("button", { name: "여행 태그 제거" })).toBeInTheDocument()
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument()
    expect(record).not.toHaveBeenCalled()
    expect(useSearchStore.getState().query).toBe("") // 질의 확정(커밋)이 일어나지 않았다 — 디바운스 전
  })

  it("T-ui-searchtag-05 [Happy] chip add applies to the filter immediately without the text debounce", async () => {
    // Arrange
    const { box } = await setup()
    await typeToken(box, "#바")
    // Act
    pressKey(box, "Enter")
    // Assert: 시간이 흐르지 않았는데도 반영
    expect(useSearchStore.getState().filters.tagIds).toEqual([1])
  })

  it("T-ui-searchtag-06 [Happy] Tab picks the highlighted tag", async () => {
    // Arrange
    const { box } = await setup()
    await typeToken(box, "#여름")
    // Act
    pressKey(box, "Tab")
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([2])
    expect(box).toHaveValue("")
  })

  it("T-ui-searchtag-07 [Happy] clicking an option picks it", async () => {
    // Arrange
    const { box } = await setup()
    await typeToken(box, "#")
    // Act
    fireEvent.click(screen.getByRole("option", { name: /#여름/ }))
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([2])
  })

  it("T-ui-searchtag-08 [Happy] Esc closes only the popup and keeps the token text", async () => {
    // Arrange
    const { box } = await setup()
    await typeToken(box, "#여")
    // Act
    const notPrevented = pressKey(box, "Escape")
    // Assert: type="search" 의 Esc 기본 동작(값 비우기)을 막아야 한다 (regression — 실제 WebView2 에서 토큰이 지워졌음)
    expect(notPrevented).toBe(false)
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument()
    expect(box).toHaveAttribute("aria-expanded", "false")
    expect(box).toHaveValue("#여")
    expect(useSearchStore.getState().filters.tagIds).toEqual([])
    // Act: 입력을 이어가면 다시 열린다
    fireEvent.change(box, { target: { value: "#여름" } })
    expect(screen.getByRole("listbox")).toBeInTheDocument()
  })

  it("T-ui-searchtag-09 [Boundary] # glued to a word does not open the popup", async () => {
    // Arrange
    const { box } = await setup()
    // Act
    await typeToken(box, "abc#여")
    // Assert
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument()
    expect(box).toHaveAttribute("aria-expanded", "false")
  })

  it("T-ui-searchtag-10 [Business rule] already chosen tags are not suggested again", async () => {
    // Arrange
    const { box } = await setup()
    await typeToken(box, "#여름")
    pressKey(box, "Enter")
    // Act
    await typeToken(box, "#여")
    // Assert
    expect(optionNames()).toEqual(["#여행57", "#가족여행12"])
  })
})

describe("SearchBox tag chips", () => {
  const withChips = async (ids: number[]) => {
    const ctx = await setup()
    act(() => useSearchStore.getState().patchFilters({ tagIds: ids }))
    await advance(0)
    return ctx
  }

  it("T-ui-searchtag-11 [Happy] chips render from filters.tagIds with resolved names; × removes one immediately", async () => {
    // Arrange
    await withChips([1, 2])
    const list = screen.getByRole("list", { name: "선택한 태그" })
    expect(within(list).getAllByRole("listitem").map((x) => x.textContent)).toEqual(["#바다", "#여름"])
    // Act
    fireEvent.click(screen.getByRole("button", { name: "바다 태그 제거" }))
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([2])
  })

  it("T-ui-searchtag-12 [Happy] Backspace on an empty input removes the last chip; with text it does not", async () => {
    // Arrange
    const { box } = await withChips([1, 2])
    fireEvent.change(box, { target: { value: "a" } })
    // Act + Assert: 텍스트가 있으면 칩 유지
    pressKey(box, "Backspace")
    expect(useSearchStore.getState().filters.tagIds).toEqual([1, 2])
    // Act: 비우면 마지막 칩 제거
    fireEvent.change(box, { target: { value: "" } })
    pressKey(box, "Backspace")
    expect(useSearchStore.getState().filters.tagIds).toEqual([1])
  })

  it("T-ui-searchtag-13 [Validation] IME composing Enter/arrows are ignored — no pick, no commit", async () => {
    // Arrange
    const { box, record } = await setup()
    await typeToken(box, "#여")
    // Act
    pressKey(box, "Enter", { isComposing: true })
    pressKey(box, "ArrowDown", { isComposing: true })
    await advance(0)
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([])
    expect(screen.getAllByRole("option")[0]).toHaveAttribute("aria-selected", "true")
    expect(record).not.toHaveBeenCalled()
  })

  it("T-ui-searchtag-21 [Validation] IME composing Enter without a tag popup still commits the plain search (regression)", async () => {
    // Arrange
    const { box, record } = await setup()
    await typeToken(box, "바다")
    // Act
    pressKey(box, "Enter", { isComposing: true })
    await advance(0)
    // Assert
    expect(useSearchStore.getState().query).toBe("바다")
    expect(record).toHaveBeenCalledWith("바다", expect.anything())
  })

  it("T-ui-searchtag-14 [Business rule] no match + Enter converts an exact tag name to a chip, even when its asset count is 0", async () => {
    // Arrange
    const { box, record } = await setup()
    await typeToken(box, "#빈태그")
    expect(screen.getByText("일치하는 태그가 없습니다")).toBeInTheDocument()
    // Act
    pressKey(box, "Enter")
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([5])
    expect(box).toHaveValue("")
    expect(record).not.toHaveBeenCalled()
  })

  it("T-ui-searchtag-22 [Time] an unfinished #token is not searched while the popup is open; after Esc it is (regression)", async () => {
    // Arrange
    const { box } = await setup()
    await typeToken(box, "노을")
    await advance(300)
    expect(useSearchStore.getState().query).toBe("노을")
    // Act: 토큰 입력 중 — 디바운스가 지나도 이전 질의 유지
    fireEvent.change(box, { target: { value: "노을 #여" } })
    await advance(1000)
    // Assert
    expect(useSearchStore.getState().query).toBe("노을")
    expect(box).toHaveValue("노을 #여")
    // Act: Esc 로 팝업을 닫으면 텍스트 그대로 검색
    pressKey(box, "Escape")
    await advance(300)
    // Assert
    expect(useSearchStore.getState().query).toBe("노을 #여")
    expect(box).toHaveValue("노을 #여")
  })

  it("T-ui-searchtag-15 [Business rule] no match + Enter keeps the text and runs a plain keyword search", async () => {
    // Arrange
    const { box, record } = await setup()
    await typeToken(box, "#없는태그")
    // Act
    pressKey(box, "Enter")
    await advance(0)
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([])
    expect(box).toHaveValue("#없는태그")
    expect(useSearchStore.getState().query).toBe("#없는태그")
    expect(record).toHaveBeenCalledTimes(1)
  })

  it("T-ui-searchtag-16 [Business rule] patched tag filters render chips; resetFilters clears them", async () => {
    // Arrange
    await setup()
    // Act
    act(() => useSearchStore.getState().patchFilters({ tagIds: [1, 3] }))
    // Assert
    expect(screen.getByRole("button", { name: "바다 태그 제거" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "여행 태그 제거" })).toBeInTheDocument()
    // Act
    act(() => useSearchStore.getState().resetFilters())
    // Assert
    expect(screen.queryByRole("list", { name: "선택한 태그" })).not.toBeInTheDocument()
  })

  it("T-ui-searchtag-17 [Business rule] tag ids missing from the loaded list are pruned from the filter", async () => {
    // Arrange + Act
    await withChips([1, 999])
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toEqual([1])
  })

  it("T-ui-searchtag-18 [Boundary] at 50 chips suggestions are replaced by a notice and Enter commits the text search", async () => {
    // Arrange: 50개 태그 + 전부 선택
    const many: Tag[] = Array.from({ length: 50 }, (_, i) => ({ tagId: i + 1, name: `t${i + 1}`, assetCount: 5 }))
    const { box } = await setup(async () => many)
    act(() => useSearchStore.getState().patchFilters({ tagIds: many.map((x) => x.tagId) }))
    await advance(0)
    // Act
    await typeToken(box, "#t")
    // Assert
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument()
    expect(screen.getByText("태그는 최대 50개까지 고를 수 있습니다")).toBeInTheDocument()
    // Act
    pressKey(box, "Enter")
    await advance(0)
    // Assert
    expect(useSearchStore.getState().filters.tagIds).toHaveLength(50)
    expect(useSearchStore.getState().query).toBe("#t")
  })

  it("T-ui-searchtag-19 [External] listTags failure shows no popup, keeps working as a plain search box and keeps the filter untouched", async () => {
    // Arrange
    const { box } = await setup(async () => {
      throw new Error("boom")
    })
    act(() => useSearchStore.getState().patchFilters({ tagIds: [7] }))
    // Act
    await typeToken(box, "#여")
    pressKey(box, "Enter")
    await advance(0)
    // Assert: 목록을 못 받았으니 정리하지 않고, 이름 대신 대체 라벨로 칩을 보인다
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument()
    expect(useSearchStore.getState().filters.tagIds).toEqual([7])
    expect(screen.getByRole("button", { name: "태그 7 태그 제거" })).toBeInTheDocument()
    expect(useSearchStore.getState().query).toBe("#여")
  })

  it("T-ui-searchtag-20 [Business rule] the recent-search dropdown and the tag popup are mutually exclusive", async () => {
    // Arrange: 최근 검색이 있는 상태
    const api = createMockApi({ count: 10 })
    await api.recordRecentSearch("old query", DEFAULT_FILTERS)
    vi.spyOn(api, "listTags").mockResolvedValue(TAGS)
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
    render(
      <ApiProvider api={api}>
        <QueryClientProvider client={queryClient}>
          <SearchBox />
        </QueryClientProvider>
      </ApiProvider>
    )
    const box = screen.getByRole("combobox", { name: "검색" })
    await advance(300)
    // Act: 빈 입력 포커스 → 최근 검색만
    fireEvent.focus(box)
    await advance(0)
    expect(screen.getByRole("list", { name: "최근 검색" })).toBeInTheDocument()
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument()
    // Act: # 입력 → 태그 팝업만
    fireEvent.change(box, { target: { value: "#" } })
    // Assert
    expect(screen.queryByRole("list", { name: "최근 검색" })).not.toBeInTheDocument()
    expect(screen.getByRole("listbox")).toBeInTheDocument()
  })
})
