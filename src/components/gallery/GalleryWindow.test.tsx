// Categories: Happy (T-ui-gwin-01, 10, 11) / Boundary (T-ui-gwin-02, 15, 16) / Validation (T-ui-gwin-12) /
// Business rule (T-ui-gwin-03, 13, 14, 17, 18, 19) / Concurrency (T-ui-gwin-20, 21)
// 갤러리는 오프셋 스크롤이라 jsdom 에서는 "보이는 행의 translateY" 와 접근성 계약(role/aria/text)으로 확인한다.
// 레이아웃 스텁: 폭 1000 / 높이 600, 크기 M → 5열, 카드 188.8 + 정보 52 + 간격 8 = 행 248.8, 월 머리글 36
import { QueryClient } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import App from "@/App"
import { qk } from "@/hooks/keys"
import type { UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary, GalleryPaging } from "@/lib/ipc-contract"
import { resetAllStores, useSelectionStore } from "@/stores"
import { stubLayout } from "@/test/layout"
import { optionNamed } from "@/test/queries"
import { fakeAssets, mockFixedResults } from "@/test/searchMock"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const ROW_H = 248.8

async function setup(items: AssetSummary[], opts: { paging?: GalleryPaging; pageSize?: 50 | 100 | 200; estimate?: boolean; generation?: { value: number } } = {}) {
  stubLayout({ width: 1000, height: 600 })
  const api: UiApi = createMockApi({ count: 10 })
  await api.updateSettings({ galleryPaging: opts.paging ?? "infinite", galleryPageSize: opts.pageSize ?? 100 })
  const mocks = mockFixedResults(api, items, { totalIsEstimate: opts.estimate, generation: opts.generation })
  const qc = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  resetAllStores()
  const user = userEvent.setup()
  render(<App api={api} queryClient={qc} />)
  await screen.findAllByRole("option")
  const box = screen.getByRole("listbox", { name: "미디어 목록" })
  const opt = (id: number) => screen.getByRole("option", { name: optionNamed(`file_${id}.jpg`) })
  const optOrNull = (id: number) => screen.queryByRole("option", { name: optionNamed(`file_${id}.jpg`) })
  /** 항목이 든 행의 translateY(px) */
  const rowTop = (id: number) => parseFloat(/translateY\((-?[\d.]+)px\)/.exec(opt(id).closest<HTMLElement>("[style*=translateY]")?.style.transform ?? "")?.[1] ?? "NaN")
  const headerTop = (label: string) =>
    parseFloat(/translateY\((-?[\d.]+)px\)/.exec(screen.getByText(label).closest<HTMLElement>("[style*=translateY]")?.style.transform ?? "")?.[1] ?? "NaN")
  const placeholders = () => document.querySelectorAll("[data-placeholder]").length
  const selectedIds = () => [...useSelectionStore.getState().ids].sort((a, b) => a - b)
  return { api, qc, user, box, opt, optOrNull, rowTop, headerTop, placeholders, selectedIds, ...mocks }
}

describe("infinite scroll (offset scrolling)", () => {
  it("T-ui-gwin-01 [Happy] a wheel jump shows skeleton cells first, requests only the visible block, then renders the cells", async () => {
    // Arrange
    const { box, search, optOrNull, opt, placeholders } = await setup(fakeAssets(5000))
    search.mockClear()
    // Act: 행 200 (= 항목 1000) 근처로 한 번에 — 머리글 36 + 200 행
    fireEvent.wheel(box, { deltaY: 36 + 200 * ROW_H + 10 })
    // Assert: 요청 전에는 스켈레톤 (옵션 없음)
    expect(screen.queryAllByRole("option")).toHaveLength(0)
    expect(placeholders()).toBeGreaterThan(0)
    // 블록 도착 후 셀
    expect(await screen.findByRole("option", { name: optionNamed("file_1001.jpg") })).toBeInTheDocument()
    expect(optOrNull(1)).toBeNull()
    expect(opt(1001)).toBeInTheDocument()
    // 중간 위치(블록 5 등)는 요청하지 않았다 — 도착지 주변 블록(9~11)뿐
    const blocks = search.mock.calls.map(([r]) => (r.at !== undefined ? r.at / 100 : null)).filter((b) => b !== null)
    expect(blocks.length).toBeGreaterThan(0)
    expect(blocks.every((b) => b >= 9 && b <= 11)).toBe(true)
  })

  it("T-ui-gwin-02 [Boundary] wheel lines scale by 40px, clamp at the top, and never scroll above the first header", async () => {
    // Arrange
    const { box, headerTop } = await setup(fakeAssets(500))
    expect(headerTop("2026년 10월")).toBe(0)
    // Act: 3줄 = 120px
    fireEvent.wheel(box, { deltaY: 3, deltaMode: 1 })
    // Assert
    expect(headerTop("2026년 10월")).toBe(-120)
    // Act: 위로 크게 — 0 에서 멈춘다
    fireEvent.wheel(box, { deltaY: -5000 })
    expect(headerTop("2026년 10월")).toBe(0)
  })

  it("T-ui-gwin-03 [Business rule] Ctrl+A selects only the loaded items after a long scroll, never more than 12 blocks, and the selection bar says so", async () => {
    // Arrange
    const { box, user, selectedIds } = await setup(fakeAssets(6000))
    // Act: 서로 다른 위치로 여러 번 이동하며 각 위치의 블록을 받는다
    for (const row of [300, 20, 900, 150, 1100, 500, 40, 700, 250, 1000, 60, 800]) {
      fireEvent.wheel(box, { deltaY: 36 + row * ROW_H + 10 })
      await screen.findAllByRole("option")
    }
    box.focus()
    await user.keyboard("{Control>}a{/Control}")
    // Assert: 메모리 상한(100건 × 12 블록)을 넘지 않고, 전체(6000)가 아니다
    const n = selectedIds().length
    expect(n).toBeGreaterThan(0)
    expect(n).toBeLessThanOrEqual(1200)
    expect(screen.getByText(`불러온 ${n.toLocaleString("ko-KR")}개만 선택됨`)).toBeInTheDocument()
  })

  it("T-ui-gwin-04 [Business rule] when everything is loaded the select-all notice is not shown", async () => {
    // Arrange: 80건은 한 블록
    const { box, user, selectedIds } = await setup(fakeAssets(80))
    box.focus()
    // Act
    await user.keyboard("{Control>}a{/Control}")
    // Assert
    expect(selectedIds()).toHaveLength(80)
    expect(screen.queryByText(/개만 선택됨/)).not.toBeInTheDocument()
  })

  it("T-ui-gwin-05 [Business rule] the context menu resolves the target from loaded blocks (lookup) after scrolling", async () => {
    // Arrange
    const { box } = await setup(fakeAssets(3000))
    fireEvent.wheel(box, { deltaY: 36 + 100 * ROW_H + 10 })
    const target = await screen.findByRole("option", { name: optionNamed("file_501.jpg") })
    // Act
    fireEvent.contextMenu(target)
    // Assert: 메뉴가 열리고 우클릭 대상이 선택된다
    expect(await screen.findByRole("menuitem", { name: /미리보기 열기/ })).toBeInTheDocument()
    expect(useSelectionStore.getState().ids.has(501)).toBe(true)
    expect(target).toHaveAttribute("aria-selected", "true")
  })
})

describe("paged mode (settings gate ON)", () => {
  it("T-ui-gwin-10 [Happy] galleryPaging=paged renders the pager and only the first page; the default mode has no pager", async () => {
    // Arrange + Act
    const paged = await setup(fakeAssets(1000), { paging: "paged" })
    // Assert
    const nav = screen.getByRole("navigation", { name: "쪽 이동" })
    expect(within(nav).getByText("1–100 / 1,000건")).toBeInTheDocument()
    expect(within(nav).getByRole("button", { name: "1쪽" })).toHaveAttribute("aria-current", "page")
    expect(within(nav).getByRole("button", { name: "10쪽" })).toBeInTheDocument()
    expect(within(nav).getByRole("button", { name: "첫 쪽" })).toBeDisabled()
    expect(paged.optOrNull(1)).not.toBeNull()
    expect(paged.search.mock.calls.every(([r]) => r.limit === 100)).toBe(true)
  })

  it("T-ui-gwin-11 [Happy] the default (infinite) setting shows no pager", async () => {
    await setup(fakeAssets(300))
    expect(screen.queryByRole("navigation", { name: "쪽 이동" })).not.toBeInTheDocument()
  })

  it("T-ui-gwin-12 [Validation] the page number input rejects non-numbers and out-of-range values inline, accepts a valid one", async () => {
    // Arrange
    const { user, opt } = await setup(fakeAssets(1000), { paging: "paged" })
    const nav = screen.getByRole("navigation", { name: "쪽 이동" })
    const input = within(nav).getByRole("textbox", { name: "쪽 번호" })
    // Act + Assert: 숫자가 아님
    await user.type(input, "abc{Enter}")
    expect(input).toHaveAttribute("aria-invalid", "true")
    const message = within(nav).getByText("1–10 사이의 쪽 번호를 입력하세요")
    expect(input.getAttribute("aria-describedby")).toBe(message.id)
    // 범위 밖
    await user.clear(input)
    await user.type(input, "11{Enter}")
    expect(input).toHaveAttribute("aria-invalid", "true")
    expect(within(nav).getByText("1–100 / 1,000건")).toBeInTheDocument()
    // 입력을 고치면 오류가 사라지고, 올바른 값은 이동한다
    await user.clear(input)
    expect(input).not.toHaveAttribute("aria-invalid")
    await user.type(input, "5")
    await user.click(within(nav).getByRole("button", { name: "이동" }))
    expect(await within(nav).findByText("401–500 / 1,000건")).toBeInTheDocument()
    expect(await screen.findByRole("option", { name: optionNamed("file_401.jpg") })).toBeInTheDocument()
    expect(opt(401)).toBeInTheDocument()
  })

  it("T-ui-gwin-13 [Happy] next / last / first buttons move between pages and request the right block", async () => {
    // Arrange
    const { user, search } = await setup(fakeAssets(1000), { paging: "paged" })
    const nav = screen.getByRole("navigation", { name: "쪽 이동" })
    // Act + Assert
    await user.click(within(nav).getByRole("button", { name: "다음 쪽" }))
    expect(await within(nav).findByText("101–200 / 1,000건")).toBeInTheDocument()
    expect(await screen.findByRole("option", { name: optionNamed("file_101.jpg") })).toBeInTheDocument()
    await user.click(within(nav).getByRole("button", { name: "마지막 쪽" }))
    expect(await within(nav).findByText("901–1,000 / 1,000건")).toBeInTheDocument()
    expect(within(nav).getByRole("button", { name: "다음 쪽" })).toBeDisabled()
    await user.click(within(nav).getByRole("button", { name: "첫 쪽" }))
    expect(await within(nav).findByText("1–100 / 1,000건")).toBeInTheDocument()
    expect(search.mock.calls.length).toBeGreaterThan(1)
  })

  it("T-ui-gwin-14 [Business rule] Ctrl+PageDown / Ctrl+PageUp change the page from the list", async () => {
    // Arrange
    const { user, box } = await setup(fakeAssets(1000), { paging: "paged" })
    const nav = screen.getByRole("navigation", { name: "쪽 이동" })
    box.focus()
    // Act + Assert
    await user.keyboard("{Control>}{PageDown}{/Control}")
    expect(await within(nav).findByText("101–200 / 1,000건")).toBeInTheDocument()
    await user.keyboard("{Control>}{PageUp}{/Control}")
    expect(await within(nav).findByText("1–100 / 1,000건")).toBeInTheDocument()
  })

  it("T-ui-gwin-15 [Boundary] arrow keys past the page edge move to the next page's first item and back to the previous page's last item", async () => {
    // Arrange
    const { user, box, opt, selectedIds } = await setup(fakeAssets(1000), { paging: "paged" })
    const nav = screen.getByRole("navigation", { name: "쪽 이동" })
    await user.click(opt(1))
    // Act: 쪽 끝(100)으로 — End
    await user.keyboard("{End}")
    expect(box.getAttribute("aria-activedescendant")).toBe("gallery-opt-100")
    await user.keyboard("{ArrowRight}")
    // Assert: 다음 쪽 첫 항목
    expect(await within(nav).findByText("101–200 / 1,000건")).toBeInTheDocument()
    await waitFor(() => expect(box.getAttribute("aria-activedescendant")).toBe("gallery-opt-101"))
    expect(selectedIds()).toEqual([101])
    // Act: 쪽 처음에서 ArrowLeft -> 이전 쪽 마지막 항목
    await user.keyboard("{ArrowLeft}")
    expect(await within(nav).findByText("1–100 / 1,000건")).toBeInTheDocument()
    await waitFor(() => expect(box.getAttribute("aria-activedescendant")).toBe("gallery-opt-100"))
    expect(selectedIds()).toEqual([100])
  })

  it("T-ui-gwin-16 [Boundary] an estimated total (semantic search) hides the last-page button and shows 'about N pages'", async () => {
    // Arrange + Act
    await setup(fakeAssets(1000), { paging: "paged", estimate: true })
    // Assert
    const nav = screen.getByRole("navigation", { name: "쪽 이동" })
    expect(within(nav).getByText("약 10쪽")).toBeInTheDocument()
    expect(within(nav).getByText("1–100 / 약 1,000건")).toBeInTheDocument()
    expect(within(nav).queryByRole("button", { name: "마지막 쪽" })).not.toBeInTheDocument()
    expect(within(nav).queryByRole("button", { name: "10쪽" })).not.toBeInTheDocument()
  })

  it("T-ui-gwin-17 [Business rule] a page that starts inside a month shows the continued header, the next month a normal one", async () => {
    // Arrange: 10월 105건 + 9월 145건 — 2쪽(101~200)은 10월 중간(마지막 5건)에서 시작
    const items = [...fakeAssets(105, 1, 9), ...fakeAssets(145, 106, 8)]
    const { user, headerTop } = await setup(items, { paging: "paged" })
    expect(screen.getByText("2026년 10월")).toBeInTheDocument()
    // Act
    await user.click(within(screen.getByRole("navigation", { name: "쪽 이동" })).getByRole("button", { name: "다음 쪽" }))
    // Assert
    expect(await screen.findByText("2026년 10월 (이어짐)")).toBeInTheDocument()
    expect(headerTop("2026년 10월 (이어짐)")).toBe(0)
    expect(screen.queryByText("2026년 10월")).not.toBeInTheDocument()
    expect(screen.getByText("2026년 9월")).toBeInTheDocument()
  })

  it("T-ui-gwin-18 [Business rule] Ctrl+A selects the whole current page only, with no loaded-only notice", async () => {
    // Arrange
    const { user, box, selectedIds } = await setup(fakeAssets(1000), { paging: "paged" })
    await user.click(within(screen.getByRole("navigation", { name: "쪽 이동" })).getByRole("button", { name: "다음 쪽" }))
    await screen.findByRole("option", { name: optionNamed("file_101.jpg") })
    box.focus()
    // Act
    await user.keyboard("{Control>}a{/Control}")
    // Assert: 101..200 (이전 쪽 블록이 메모리에 남아 있어도 포함하지 않는다)
    expect(selectedIds()).toHaveLength(100)
    expect(selectedIds()[0]).toBe(101)
    expect(selectedIds()[99]).toBe(200)
    expect(screen.queryByText(/개만 선택됨/)).not.toBeInTheDocument()
  })

  it("T-ui-gwin-19 [Business rule] the list view works in paged mode too (rows + pager)", async () => {
    // Arrange
    const { user } = await setup(fakeAssets(1000), { paging: "paged" })
    // Act
    await user.click(screen.getByRole("radio", { name: "목록" }))
    // Assert
    expect(await screen.findByRole("option", { name: optionNamed("file_1.jpg") })).toHaveAttribute("data-asset-id", "1")
    expect(screen.getByRole("navigation", { name: "쪽 이동" })).toBeInTheDocument()
  })
})

describe("anchors", () => {
  it("T-ui-gwin-20 [Business rule] switching infinite -> paged keeps the first visible item: its page opens scrolled to it", async () => {
    // Arrange: 첫 보이는 항목 = 510 (행 102)
    const { api, qc, box, rowTop } = await setup(fakeAssets(1000))
    fireEvent.wheel(box, { deltaY: 36 + 102 * ROW_H + 10 })
    await screen.findByRole("option", { name: optionNamed("file_511.jpg") })
    // Act: 설정에서 페이지 모드로
    await act(async () => {
      const s = await api.updateSettings({ galleryPaging: "paged" })
      qc.setQueryData(qk.settings, s)
    })
    // Assert: 6쪽(501–600) + 510 이 든 행이 맨 위
    const nav = await screen.findByRole("navigation", { name: "쪽 이동" })
    expect(await within(nav).findByText("501–600 / 1,000건")).toBeInTheDocument()
    await screen.findByRole("option", { name: optionNamed("file_511.jpg") })
    expect(rowTop(511)).toBeCloseTo(0, 0)
  })

  it("T-ui-gwin-21 [Concurrency] a generation change during a scan restores the same global position after the new index arrives", async () => {
    // Arrange
    const generation = { value: 1 }
    const { qc, box, rowTop, search } = await setup(fakeAssets(1000), { generation })
    fireEvent.wheel(box, { deltaY: 36 + 102 * ROW_H + 10 })
    await screen.findByRole("option", { name: optionNamed("file_511.jpg") })
    search.mockClear()
    // Act: 스캔이 색인을 바꿨다 — 세대가 오른 새 색인
    generation.value = 2
    await act(async () => {
      await qc.invalidateQueries({ queryKey: qk.search })
    })
    // Assert: 블록을 다시 받아 같은 자리(항목 510 의 행이 맨 위)에 그려진다
    await waitFor(() => expect(search).toHaveBeenCalled())
    expect(await screen.findByRole("option", { name: optionNamed("file_511.jpg") })).toBeInTheDocument()
    await waitFor(() => expect(rowTop(511)).toBeCloseTo(0, 0))
  })
})
