// Categories: Happy (T-ui-topbar-01 collapse markup, 02 sort menu) / Boundary (T-ui-topbar-03 narrow search open/close, 06 overflow contract) / Business rule (T-ui-topbar-04 query badge, 05 semantic sort disabled, 07 Esc keeps viewer)
// jsdom 은 컨테이너 쿼리를 계산하지 않는다 — 접힘 단계는 "어떤 요소가 어떤 @max-[N] 클래스를 갖는가" 라는 마크업 계약으로, 펼침 입력·메뉴·배지 동작은 접근성 역할로 검증한다.
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useSearchStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

async function setup() {
  stubLayout()
  stubMedia()
  const api = createMockApi({ count: 20 })
  await useStubbedDetail(api)
  const user = userEvent.setup()
  renderApp(api)
  await screen.findAllByRole("option")
  return { api, user }
}

const topBar = (): HTMLElement => document.querySelector("[data-viewer-sibling] > header") as HTMLElement

describe("TopBar collapse (container query contract)", () => {
  it("T-ui-topbar-01 [Happy] the header is the @container and each collapse stage hangs off its own threshold (sort/filter at 640, search at 420)", async () => {
    // Arrange + Act
    await setup()
    const bar = topBar()
    // Assert: 판정 기준은 창이 아니라 헤더 컨테이너
    expect(bar.className).toContain("@container")
    // 단계 1 (< 640): 정렬 Select 숨김 + 정렬 아이콘 메뉴 노출, 필터 라벨 숨김(이름은 aria-label 로 유지)
    expect(within(bar).getByRole("combobox", { name: "정렬 기준" }).className).toContain("@max-[640px]:hidden")
    expect(within(bar).getByRole("button", { name: "정렬 기준" }).className).toContain("@max-[640px]:inline-flex")
    const filter = within(bar).getByRole("button", { name: "필터" })
    expect(filter.querySelector("span")?.className).toContain("@max-[640px]:hidden")
    // 단계 2 (< 420): 검색 박스 숨김 + 돋보기 노출
    expect(within(bar).getByTestId("top-search").className).toContain("@max-[420px]:hidden")
    expect(within(bar).getByRole("button", { name: "검색 열기" }).className).toContain("@max-[420px]:inline-flex")
  })

  it("T-ui-topbar-06 [Regression] the search input can shrink to 0 and ellipsizes its placeholder instead of overflowing onto the filter button", async () => {
    // Arrange + Act
    await setup()
    const input = within(topBar()).getByRole("combobox", { name: "검색" })
    // Assert: 예전 min-w-24(96px) 가 래퍼 밖 넘침의 원인이었다
    expect(input.className).toContain("min-w-0")
    expect(input.className).not.toContain("min-w-24")
    expect(input.className).toContain("text-ellipsis")
  })
})

describe("TopBar collapsed sort menu", () => {
  it("T-ui-topbar-02 [Happy] the icon button opens the same choices and picking one changes the sort", async () => {
    // Arrange
    const { user } = await setup()
    // Act
    await user.click(within(topBar()).getByRole("button", { name: "정렬 기준" }))
    const menu = await screen.findByRole("menu")
    // Assert: Select 와 같은 항목
    expect(within(menu).getByRole("menuitemradio", { name: "제목" })).toBeInTheDocument()
    expect(within(menu).getByRole("menuitemradio", { name: "관련도" })).toBeInTheDocument()
    // Act
    await user.click(within(menu).getByRole("menuitemradio", { name: "제목" }))
    // Assert
    expect(useSearchStore.getState().sort.field).toBe("name")
  })

  it("T-ui-topbar-05 [Business rule, gate ON] while semantic search is active the icon button is disabled like the Select and keeps the explanation", async () => {
    // Arrange
    await setup()
    // Act: 의미 검색 켜기 (하이브리드 + 질의)
    act(() => {
      useSearchStore.getState().setMode("hybrid")
      useSearchStore.getState().setQuery("sunset")
    })
    // Assert
    const button = within(topBar()).getByRole("button", { name: "정렬 기준" })
    expect(button).toBeDisabled()
    expect(within(topBar()).getByRole("combobox", { name: "정렬 기준" })).toBeDisabled()
    expect(button.getAttribute("aria-describedby")).toBe("sort-semantic-note")
  })
})

describe("TopBar collapsed search", () => {
  it("T-ui-topbar-03 [Boundary] opening focuses the same input; back, Esc and focus leaving close it without touching the query", async () => {
    // Arrange
    const { user } = await setup()
    act(() => useSearchStore.getState().setQuery("beach"))
    const bar = topBar()
    const wrap = within(bar).getByTestId("top-search")
    expect(wrap.dataset.open).toBeUndefined()
    const openButton = () => within(bar).getByRole("button", { name: /^검색 열기/ })
    // Act: 열기
    await user.click(openButton())
    // Assert: 상단 바 전체를 덮는 입력 + 포커스 + 닫기 버튼, 입력은 같은 인스턴스라 검색어 유지
    expect(wrap.dataset.open).toBe("true")
    expect(wrap.className).toContain("@max-[420px]:absolute")
    const input = within(wrap).getByRole("combobox", { name: "검색" })
    await waitFor(() => expect(input).toHaveFocus())
    expect(input).toHaveValue("beach")
    // Act: 뒤로 버튼으로 닫기
    await user.click(within(wrap).getByRole("button", { name: "검색 닫기" }))
    expect(wrap.dataset.open).toBeUndefined()
    // Act: 다시 열고 Esc
    await user.click(openButton())
    await waitFor(() => expect(input).toHaveFocus())
    await user.keyboard("{Escape}")
    // Assert: 닫힘, 입력값·검색어는 그대로 (type=search 의 Esc 기본 동작(값 비우기)을 막았다)
    expect(wrap.dataset.open).toBeUndefined()
    expect(input).toHaveValue("beach")
    expect(useSearchStore.getState().query).toBe("beach")
    // Act: 다시 열고 바깥 클릭 (포커스 이탈)
    await user.click(openButton())
    await waitFor(() => expect(input).toHaveFocus())
    await user.click(document.body)
    // Assert
    expect(wrap.dataset.open).toBeUndefined()
    expect(useSearchStore.getState().query).toBe("beach")
  })

  it("T-ui-topbar-04 [Business rule] a dot badge and tooltip show only while a query is active", async () => {
    // Arrange
    await setup()
    const bar = topBar()
    // Assert: 검색어 없음
    expect(within(bar).queryByTestId("search-active-dot")).not.toBeInTheDocument()
    expect(within(bar).getByRole("button", { name: "검색 열기" })).toHaveAttribute("title", "검색 열기")
    // Act
    act(() => useSearchStore.getState().setQuery("goat"))
    // Assert: 점 배지 + 툴팁/이름에 검색어
    expect(within(bar).getByTestId("search-active-dot")).toBeInTheDocument()
    const button = within(bar).getByRole("button", { name: "검색 열기 — 검색어: goat" })
    expect(button).toHaveAttribute("title", "검색 열기 — 검색어: goat")
    // Act: 검색어 지움
    act(() => useSearchStore.getState().setQuery(""))
    // Assert
    expect(within(bar).queryByTestId("search-active-dot")).not.toBeInTheDocument()
  })

  it("T-ui-topbar-07 [Business rule] Esc in the expanded search closes only the search - the open viewer stays", async () => {
    // Arrange
    const { user } = await setup()
    act(() => useViewStore.getState().openPreview(1))
    await screen.findByLabelText("clip.mp4", { selector: "video" })
    const bar = topBar()
    await user.click(within(bar).getByRole("button", { name: "검색 열기" }))
    const input = within(bar).getByRole("combobox", { name: "검색" })
    await waitFor(() => expect(input).toHaveFocus())
    // Act
    await user.keyboard("{Escape}")
    // Assert
    expect(within(bar).getByTestId("top-search").dataset.open).toBeUndefined()
    expect(useViewStore.getState().previewId).toBe(1)
  })
})
