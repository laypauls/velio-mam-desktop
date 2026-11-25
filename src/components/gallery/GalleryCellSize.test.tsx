// Categories: Boundary (T-ui-cellsize-01, 02, 04) / Business rule (T-ui-cellsize-03)
import { act, cleanup, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary } from "@/lib/ipc-contract"
import { useSearchStore, useViewStore } from "@/stores"
import type { CardSize } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { mockFixedResults } from "@/test/searchMock"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const GAP = 8
const PAD = 12

const asset = (id: number): AssetSummary => ({
  assetId: id,
  fileName: `file_${id}.jpg`,
  mediaType: "image",
  availability: "online",
  width: 1,
  height: 1,
  durationMs: null,
  fileSize: 1,
  capturedAt: Date.UTC(2026, 9, 5),
  modifiedAt: Date.UTC(2026, 9, 5),
  favorite: false,
  rating: 0,
  thumbState: "done",
  aiState: "none",
  title: null,
  revision: 1,
})

async function renderGrid(width: number, size: CardSize) {
  stubLayout({ width, height: 600 })
  const api = createMockApi({ count: 10 })
  const items = Array.from({ length: 40 }, (_, i) => asset(i + 1))
  mockFixedResults(api, items)
  renderApp(api)
  act(() => {
    useViewStore.getState().setCardSize(size)
    useSearchStore.getState().setSort({ field: "relevance", direction: "desc" }) // 머리글 없는 첫 행
  })
  const first = (await screen.findAllByRole("option"))[0]
  const rowCells = [...(first.parentElement?.parentElement?.children ?? [])] as HTMLElement[]
  const widths = rowCells.map((c) => parseFloat(c.style.width))
  const right = widths.reduce((sum, w) => sum + w, 0) + GAP * (widths.length - 1) // 마지막 카드 오른쪽 끝(행 왼쪽 기준)
  return { widths, count: rowCells.length, right }
}

describe("Gallery fills the row width: columns = round((inner+gap)/(target+gap)), no leftover on the right", () => {
  it.each([1000, 1100, 1180, 1276])("T-ui-cellsize-01 [Boundary] window %ipx, M: every card has the same width and the last card ends exactly at inner", async (width) => {
    // Arrange + Act
    const { widths, count, right } = await renderGrid(width, "m")
    // Assert
    const inner = width - PAD * 2
    expect(count).toBe(Math.round((inner + GAP) / (180 + GAP)))
    expect(new Set(widths).size).toBe(1)
    expect(right).toBeCloseTo(inner, 6)
  })

  it("T-ui-cellsize-02 [Boundary] a container narrower than the size gets one column that fills it instead of overflowing", async () => {
    // Arrange + Act
    const { widths, count } = await renderGrid(150, "m")
    // Assert: inner = 150 - 24
    expect(count).toBe(1)
    expect(widths).toEqual([126])
  })

  it("T-ui-cellsize-03 [Business rule] changing the size preset changes the column count and card width; every preset fills the row", async () => {
    // Arrange: inner = 1176
    const width = 1200
    const inner = width - PAD * 2
    const result: Record<string, { cols: number; cell: number }> = {}
    // Act
    for (const size of ["xs", "s", "m", "l", "xl"] as const) {
      const { widths, count, right } = await renderGrid(width, size)
      expect(right).toBeCloseTo(inner, 6)
      result[size] = { cols: count, cell: widths[0] }
      cleanup()
    }
    // Assert: 목표 120/150/180/240/300 -> 열 수 감소, 폭 증가
    expect(Object.values(result).map((r) => r.cols)).toEqual([9, 7, 6, 5, 4])
    const cells = Object.values(result).map((r) => r.cell)
    expect([...cells].sort((a, b) => a - b)).toEqual(cells)
    expect(new Set(cells).size).toBe(5)
  })
})
