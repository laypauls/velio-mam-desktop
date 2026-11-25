// Categories: Happy (T-ui-search-expand-01/02), Boundary (T-ui-search-expand-04), Business rule (T-ui-search-expand-03/05/06)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { Tag } from "@/lib/ipc-contract"
import { resetAllStores } from "@/stores"
import { SearchBox } from "./SearchBox"

const TAGS: Tag[] = [{ tagId: 1, name: "바다", assetCount: 400 }]

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date"] })
  resetAllStores()
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
})

const advance = (ms: number) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)))

async function setup() {
  const api = createMockApi({ count: 10 })
  vi.spyOn(api, "listTags").mockImplementation(async () => TAGS)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <SearchBox />
      </QueryClientProvider>
    </ApiProvider>
  )
  const input = screen.getByRole("combobox", { name: "검색" })
  await advance(300)
  return { input, root: screen.getByTestId("search-box"), toggle: screen.getByRole("button", { name: "의미" }) }
}

const expanded = (root: HTMLElement) => root.getAttribute("data-expanded")

describe("SearchBox focus expand", () => {
  it("T-ui-search-expand-01 [Happy] idle box is not expanded", async () => {
    const { root } = await setup()
    expect(expanded(root)).toBe("false")
  })

  it("T-ui-search-expand-02 [Happy] focusing the input expands the box", async () => {
    const { root, input } = await setup()
    fireEvent.focus(input)
    expect(expanded(root)).toBe("true")
  })

  it("T-ui-search-expand-03 [Business rule] focus moving to the mode toggle keeps it expanded; leaving the box collapses it", async () => {
    // Arrange
    const { root, input, toggle } = await setup()
    fireEvent.focus(input)
    // Act: 입력 → 토글로 포커스 이동 (focusout 의 relatedTarget 이 박스 안)
    fireEvent.blur(input, { relatedTarget: toggle })
    fireEvent.focus(toggle)
    // Assert
    expect(expanded(root)).toBe("true")
    // Act 2: 박스 밖으로 이동
    fireEvent.blur(toggle, { relatedTarget: document.body })
    // Assert 2
    expect(expanded(root)).toBe("false")
  })

  it("T-ui-search-expand-04 [Boundary] blur with empty text (and whitespace-only text) collapses", async () => {
    const { root, input } = await setup()
    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: "   " } })
    fireEvent.blur(input, { relatedTarget: null })
    expect(expanded(root)).toBe("false")
  })

  it("T-ui-search-expand-05 [Business rule] blur with search text stays expanded, and collapses once the text is cleared", async () => {
    const { root, input } = await setup()
    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: "노을" } })
    fireEvent.blur(input, { relatedTarget: null })
    expect(expanded(root)).toBe("true")
    fireEvent.change(input, { target: { value: "" } })
    expect(expanded(root)).toBe("false")
  })

  it("T-ui-search-expand-06 [Business rule] blur with a tag chip stays expanded", async () => {
    // Arrange: `#바` + Enter 로 칩 추가 후 텍스트는 비운다
    const { root, input } = await setup()
    fireEvent.focus(input)
    fireEvent.change(input, { target: { value: "#바" } })
    await advance(0)
    fireEvent.keyDown(input, { key: "Enter" })
    await advance(0)
    expect(screen.getByRole("button", { name: "바다 태그 제거" })).toBeInTheDocument()
    expect(input).toHaveValue("")
    // Act
    fireEvent.blur(input, { relatedTarget: null })
    // Assert
    expect(expanded(root)).toBe("true")
  })
})
