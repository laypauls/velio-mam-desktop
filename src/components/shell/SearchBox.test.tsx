// Categories: Time (T-ui-search-03 debounce) / Happy (Enter, recent selection)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { DEFAULT_FILTERS, resetAllStores, useSearchStore } from "@/stores"
import { SearchBox, SEARCH_DEBOUNCE_MS } from "./SearchBox"

beforeEach(() => {
  // setImmediate/rAF 는 가짜로 만들지 않는다 — React act 가 setImmediate 로 큐를 비운다
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] })
  resetAllStores()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

/** 확정 질의(store.query)가 실제로 바뀐 값을 순서대로 기록한다. */
function recordQueryChanges(): string[] {
  const changes: string[] = []
  useSearchStore.subscribe((s, prev) => {
    if (s.query !== prev.query) changes.push(s.query)
  })
  return changes
}

async function setup(recent: string[] = []) {
  const api = createMockApi({ count: 10 })
  for (const q of recent) await api.recordRecentSearch(q, DEFAULT_FILTERS)
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
  return { box, record }
}

const advance = (ms: number) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)))

describe("SearchBox", () => {
  it("T-ui-search-03 [Time] typing fires exactly one search after the 250ms debounce", async () => {
    // Arrange
    const { box } = await setup()
    await advance(SEARCH_DEBOUNCE_MS + 1) // 마운트 시 초기 디바운스 소진
    const changes = recordQueryChanges()
    // Act: 글자 사이 간격은 디바운스보다 짧다
    for (const v of ["b", "bu", "bus", "busa", "busan"]) {
      fireEvent.change(box, { target: { value: v } })
      await advance(50)
    }
    // Assert: 마지막 입력 직후에는 아직 반영 전
    await advance(SEARCH_DEBOUNCE_MS - 60) // 마지막 글자 후 이미 50ms 경과 → 총 240ms
    expect(changes).toEqual([])
    await advance(20)
    expect(changes).toEqual(["busan"])
    await advance(1000)
    expect(changes).toEqual(["busan"])
  })

  it("T-ui-search-04 [Happy] Enter searches immediately and records a recent search; an empty Enter records nothing", async () => {
    // Arrange
    const { box, record } = await setup()
    await advance(SEARCH_DEBOUNCE_MS + 1)
    const changes = recordQueryChanges()
    // Act
    fireEvent.change(box, { target: { value: "  sunset  " } })
    fireEvent.keyDown(box, { key: "Enter" })
    await advance(0)
    // Assert: 디바운스를 기다리지 않고 trim 된 질의가 즉시 반영
    expect(changes).toEqual(["sunset"])
    expect(record).toHaveBeenCalledTimes(1)
    expect(record.mock.calls[0][0]).toBe("sunset")
    // Act: 입력을 비우고 Enter — 기록하지 않는다
    fireEvent.change(box, { target: { value: "" } })
    fireEvent.keyDown(box, { key: "Enter" })
    await advance(0)
    // Assert
    expect(record).toHaveBeenCalledTimes(1)
  })

  it("T-ui-search-05 [Happy] picking a recent search applies the query and records it again", async () => {
    // Arrange: 입력이 비어 있고 포커스가 있을 때만 최근 검색이 열린다
    const { box, record } = await setup(["old query", "sunset"])
    await advance(SEARCH_DEBOUNCE_MS + 1)
    const changes = recordQueryChanges()
    // Act
    fireEvent.focus(box)
    await advance(0)
    const item = screen.getByRole("button", { name: "old query" })
    fireEvent.click(item)
    await advance(0)
    // Assert
    expect(changes).toEqual(["old query"])
    expect(box).toHaveValue("old query")
    expect(record).toHaveBeenCalledWith("old query", expect.anything())
    expect(screen.queryByRole("list", { name: "최근 검색" })).not.toBeInTheDocument() // 입력이 생기면 닫힌다
  })
})

// Categories: Happy (focus shows icon) / Boundary (initial, blur) / Business rule (D3 hidden when unfocused even with text)
describe("SearchBox search icon visibility", () => {
  const icon = () => screen.getByTestId("search-icon")

  it("T-ui-search-icon-01 [Boundary] icon is hidden before the input has focus", async () => {
    // Arrange / Act
    await setup()
    // Assert
    expect(icon()).toHaveAttribute("data-visible", "false")
  })

  it("T-ui-search-icon-02 [Happy] focusing the input shows the icon and blur hides it", async () => {
    // Arrange
    const { box } = await setup()
    // Act
    fireEvent.focus(box)
    // Assert
    expect(icon()).toHaveAttribute("data-visible", "true")
    // Act
    fireEvent.blur(box)
    // Assert
    expect(icon()).toHaveAttribute("data-visible", "false")
  })

  it("T-ui-search-icon-03 [Business rule] icon is hidden after blur even when text remains", async () => {
    // Arrange
    const { box } = await setup()
    fireEvent.focus(box)
    fireEvent.change(box, { target: { value: "busan" } })
    expect(icon()).toHaveAttribute("data-visible", "true")
    // Act
    fireEvent.blur(box)
    // Assert
    expect(box).toHaveValue("busan")
    expect(icon()).toHaveAttribute("data-visible", "false")
  })

  it("T-ui-search-icon-04 [Happy] input text starts at the icon when unfocused and after it when focused", async () => {
    // Arrange
    const { box } = await setup()
    // Assert: unfocused -> text starts where the (hidden) icon starts
    expect(box).toHaveAttribute("data-icon-visible", "false")
    // Act
    fireEvent.focus(box)
    // Assert: focused -> text starts after the visible icon
    expect(box).toHaveAttribute("data-icon-visible", "true")
    expect(icon()).toHaveAttribute("data-visible", "true")
    // Act
    fireEvent.blur(box)
    // Assert
    expect(box).toHaveAttribute("data-icon-visible", "false")
  })
})
