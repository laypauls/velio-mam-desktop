// Categories: Happy (T-ui-kbd-01) / Business rule (T-ui-kbd-02, T-ui-kbd-03) / Boundary (T-ui-kbd-04, T-ui-kbd-05)
// 갤러리는 오프셋 스크롤 — 스크롤 위치는 보이는 행의 translateY 로 확인한다 (jsdom 은 레이아웃이 없다)
import { act, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import { useSelectionStore, useViewStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { optionNamed } from "@/test/queries"
import { fakeAssets, mockFixedResults } from "@/test/searchMock"
import { nextIndex } from "./navigation"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const asset = (id: number, month: number): AssetSummary => ({
  assetId: id,
  fileName: `file_${id}.jpg`,
  mediaType: "image",
  availability: "online",
  width: 100,
  height: 100,
  durationMs: null,
  fileSize: 1000,
  capturedAt: Date.UTC(2026, month, 15),
  modifiedAt: Date.UTC(2026, month, 15),
  favorite: false,
  rating: 0,
  thumbState: "done",
  aiState: "none",
  title: null,
  revision: 1,
})

/** 10월 5건(1~5) + 9월 3건(6~8), 썸네일 300px → 1000px 폭에서 3열.
 *  행: 0 머리글(10월) / 1 [1,2,3] / 2 [4,5] / 3 머리글(9월) / 4 [6,7,8] */
const ITEMS = [1, 2, 3, 4, 5].map((i) => asset(i, 9)).concat([6, 7, 8].map((i) => asset(i, 8)))

async function setup({ items = ITEMS }: { items?: AssetSummary[] } = {}) {
  stubLayout({ width: 1000, height: 600 })
  const api = createMockApi({ count: 10 })
  const { search: spy } = mockFixedResults(api, items)
  const user = userEvent.setup()
  renderApp(api)
  act(() => useViewStore.getState().setCardSize("xl"))
  await screen.findAllByRole("option")
  const box = screen.getByRole("listbox", { name: "미디어 목록" })
  const opt = (id: number) => screen.getByRole("option", { name: optionNamed(`file_${id}.jpg`) })
  const selectedIds = () => [...useSelectionStore.getState().ids].sort((a, b) => a - b)
  const activeId = () => box.getAttribute("aria-activedescendant")
  return { user, box, opt, selectedIds, activeId, spy }
}

describe("Gallery keyboard navigation", () => {
  it("T-ui-kbd-01 [Happy] arrows move the active item by 1 and by column count, selecting it and exposing aria-activedescendant", async () => {
    // Arrange
    const { user, opt, activeId, selectedIds, box } = await setup()
    await user.click(opt(1)) // 클릭이 listbox 로 포커스를 준다
    expect(box).toHaveFocus()
    // Act + Assert: 좌우 1칸
    await user.keyboard("{ArrowRight}")
    expect(activeId()).toBe("gallery-opt-2")
    expect(selectedIds()).toEqual([2])
    expect(opt(2)).toHaveAttribute("aria-selected", "true")
    // 상하 열 수(3)만큼
    await user.keyboard("{ArrowDown}")
    expect(activeId()).toBe("gallery-opt-5")
    await user.keyboard("{ArrowUp}")
    expect(activeId()).toBe("gallery-opt-2")
    await user.keyboard("{ArrowLeft}{ArrowLeft}") // 첫 항목에서 멈춤
    expect(activeId()).toBe("gallery-opt-1")
    // Home / End
    await user.keyboard("{End}")
    expect(activeId()).toBe("gallery-opt-8")
    await user.keyboard("{Home}")
    expect(activeId()).toBe("gallery-opt-1")
    // Enter 는 키보드 포커스 항목의 미리보기를 연다
    await user.keyboard("{ArrowRight}")
    await user.keyboard("{Enter}")
    expect(useViewStore.getState().previewId).toBe(2)
  })

  it("T-ui-kbd-02 [Business rule] Shift+Arrow extends the range from the anchor; Ctrl+Arrow moves focus only; Space toggles", async () => {
    // Arrange
    const { user, opt, selectedIds, activeId } = await setup()
    await user.click(opt(1))
    // Act: Shift 확장
    await user.keyboard("{Shift>}{ArrowRight}{ArrowRight}{/Shift}")
    // Assert
    expect(selectedIds()).toEqual([1, 2, 3])
    await user.keyboard("{Shift>}{ArrowDown}{/Shift}") // 3 -> 6
    expect(selectedIds()).toEqual([1, 2, 3, 4, 5, 6])
    await user.keyboard("{Shift>}{ArrowLeft}{/Shift}") // 기준점(1)에서 5 까지로 축소
    expect(selectedIds()).toEqual([1, 2, 3, 4, 5])
    // Act: Ctrl+Arrow 는 선택 불변
    await user.keyboard("{Control>}{ArrowRight}{/Control}")
    expect(activeId()).toBe("gallery-opt-6")
    expect(selectedIds()).toEqual([1, 2, 3, 4, 5])
    // Act: Space 로 포커스 항목 토글(추가 → 제거)
    await user.keyboard(" ")
    expect(selectedIds()).toEqual([1, 2, 3, 4, 5, 6])
    await user.keyboard(" ")
    expect(selectedIds()).toEqual([1, 2, 3, 4, 5])
  })

  it("T-ui-kbd-03 [Business rule] moving onto the first item of the next month scrolls so its month header is visible; Home returns to the first header", async () => {
    // Arrange: 10월 헤더(0) · 행 380 · 행 380 · 9월 헤더(796) — 뷰포트 600 에서는 9월 머리글이 보이지 않는다
    const { user, opt } = await setup()
    const headerTop = (label: string) => {
      const row = screen.getByText(label).closest<HTMLElement>("[style*=translateY]")
      return parseFloat(/translateY\((-?[\d.]+)px\)/.exec(row?.style.transform ?? "")?.[1] ?? "NaN")
    }
    await user.click(opt(5))
    expect(headerTop("2026년 9월")).toBeGreaterThan(600 - 36)
    // Act: 5 -> 6 은 9월의 첫 항목
    await user.keyboard("{ArrowRight}")
    // Assert: 머리글이 화면 안으로 들어왔다 (머리글까지 포함해 보이게 이동)
    expect(headerTop("2026년 9월")).toBeGreaterThanOrEqual(0)
    expect(headerTop("2026년 9월")).toBeLessThan(600 - 36)
    // Act: Home -> 맨 위
    await user.keyboard("{Home}")
    // Assert
    expect(headerTop("2026년 10월")).toBe(0)
  })

  it("T-ui-kbd-04 [Boundary] End on a long result requests the far block, then focuses and selects the last item once it arrives", async () => {
    // Arrange: 450건 — 마지막 항목(450)의 블록은 처음에 없다
    const { user, opt, activeId, spy, selectedIds } = await setup({ items: fakeAssets(450) })
    await user.click(opt(1))
    spy.mockClear()
    // Act
    await user.keyboard("{End}")
    // Assert: 도착 후 적용 (마지막 블록은 seek)
    await waitFor(() => expect(activeId()).toBe("gallery-opt-450"))
    expect(selectedIds()).toEqual([450])
    expect(spy.mock.calls.some(([r]) => r.at !== undefined)).toBe(true)
    expect(screen.getByRole("option", { name: optionNamed("file_450.jpg") })).toHaveAttribute("aria-selected", "true")
  })

  it("T-ui-kbd-05 [Boundary] nextIndex pure rules: list ignores Left/Right, page size, partial last row, empty list, no active item", () => {
    // Arrange
    const grid = { cols: 4, pageSize: 8, horizontal: true }
    const list = { cols: 1, pageSize: 5, horizontal: false }
    // Act + Assert
    expect(nextIndex("ArrowRight", 3, 10, list)).toBeNull()
    expect(nextIndex("ArrowDown", 3, 10, list)).toBe(4)
    expect(nextIndex("PageDown", 3, 10, list)).toBe(8)
    expect(nextIndex("PageDown", 8, 10, list)).toBe(9)
    expect(nextIndex("PageUp", 3, 10, list)).toBe(0)
    expect(nextIndex("PageDown", 1, 30, grid)).toBe(9)
    expect(nextIndex("ArrowDown", 5, 10, grid)).toBe(9)
    expect(nextIndex("ArrowDown", 9, 10, grid)).toBe(9)
    expect(nextIndex("ArrowUp", 2, 10, grid)).toBe(2)
    expect(nextIndex("ArrowRight", -1, 10, grid)).toBe(0)
    expect(nextIndex("End", -1, 10, grid)).toBe(9)
    expect(nextIndex("ArrowDown", 0, 0, grid)).toBeNull()
    expect(nextIndex("a", 0, 10, grid)).toBeNull()
  })
})

// Categories: Boundary (T-ui-kbd-06) / Business rule (T-ui-kbd-07)
describe("Gallery row height follows the card info lines", () => {
  const MANY = Array.from({ length: 40 }, (_, i) => asset(i + 1, 9))
  const CELL = (1000 - 24 - 8 * 4) / 5 // 폭 채움 격자의 카드 폭

  /** 크기 M(180) / 폭 1000 → 5열, 폭 채움으로 셀 폭 188.8. 높이 700: 2줄(240.8+8)이면 2행, 1줄(220.8+8)·0줄(188.8+8)이면 3행이 보인다 */
  async function setupMany(preset: "off" | "compact" | "detailed", height = 700) {
    stubLayout({ width: 1000, height })
    const api = createMockApi({ count: 10 })
    mockFixedResults(api, MANY)
    const user = userEvent.setup()
    renderApp(api)
    act(() => {
      useViewStore.getState().setCardSize("m")
      useViewStore.getState().applyCardPreset(preset)
    })
    await screen.findAllByRole("option")
    const box = screen.getByRole("listbox", { name: "미디어 목록" })
    const opt = (id: number) => screen.getByRole("option", { name: optionNamed(`file_${id}.jpg`) })
    return { user, box, opt, activeId: () => box.getAttribute("aria-activedescendant") }
  }

  it.each([
    ["off", CELL],
    ["compact", CELL + 32],
    ["detailed", CELL + 52],
  ] as const)("T-ui-kbd-06 [Boundary] %s preset: card height is cell width + info height (%d px) and every card in the row is equal", async (preset, expected) => {
    // Arrange + Act
    const { opt } = await setupMany(preset)
    // Assert: 셀 래퍼 높이 (같은 행의 카드는 모두 같다)
    const heights = [1, 2, 3, 4, 5].map((id) => opt(id).parentElement?.style.height)
    expect(new Set(heights)).toEqual(new Set([`${expected}px`]))
  })

  it.each([
    ["detailed", "gallery-opt-11"], // 2줄: 700 / 240 → 2행 × 5열
    ["compact", "gallery-opt-16"], // 1줄: 700 / 220 → 3행 × 5열
    ["off", "gallery-opt-16"], // 0줄: 700 / 196.8 → 3행 × 5열
  ] as const)("T-ui-kbd-07 [Business rule] %s preset: PageDown jumps by the visible rows of the taller card (-> %s)", async (preset, expectedId) => {
    // Arrange
    const { user, opt, box, activeId } = await setupMany(preset)
    await user.click(opt(1))
    expect(box).toHaveFocus()
    // Act
    await user.keyboard("{PageDown}")
    // Assert
    expect(activeId()).toBe(expectedId)
  })

  it("T-ui-kbd-08 [Business rule] changing a display option while an item is active keeps the active item and re-registers the page size", async () => {
    // Arrange
    const { user, opt, box, activeId } = await setupMany("detailed")
    await user.click(opt(3))
    expect(activeId()).toBe("gallery-opt-3")
    // Act: 정보 영역을 끄면 행이 낮아져 한 페이지가 3행으로 늘어난다
    act(() => useViewStore.getState().applyCardPreset("off"))
    box.focus()
    await user.keyboard("{PageDown}")
    // Assert: 활성 항목(3)에서 출발해 3행(15칸)만큼 이동 — 선택도 따라간다
    expect(activeId()).toBe("gallery-opt-18")
    expect([...useSelectionStore.getState().ids]).toEqual([18])
  })
})
