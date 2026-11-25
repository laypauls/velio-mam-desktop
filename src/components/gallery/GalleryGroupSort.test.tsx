// Categories: Business rule (T-ui-gsort-01)
import { act, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { AssetSummary, SearchIndex } from "@/lib/ipc-contract"
import { useSearchStore } from "@/stores"
import { renderApp, stubLayout } from "@/test/layout"
import { mockFixedResults } from "@/test/searchMock"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const asset = (id: number, captured: Date, modified: Date): AssetSummary => ({
  assetId: id,
  fileName: `file_${id}.jpg`,
  mediaType: "image",
  availability: "online",
  width: 1,
  height: 1,
  durationMs: null,
  fileSize: 1,
  capturedAt: captured.getTime(),
  modifiedAt: modified.getTime(),
  favorite: false,
  rating: 0,
  thumbState: "done",
  aiState: "none",
  title: null,
  revision: 1,
})

const NOV = new Date(2026, 10, 5)
const OCT = new Date(2026, 9, 5)
// 촬영일순: 3건 모두 촬영 11월, 수정일은 11 / 10 / 11 → 수정일로 묶으면 11월이 떨어져 두 번 나온다
const BY_CAPTURED = [asset(1, NOV, NOV), asset(2, NOV, OCT), asset(3, NOV, NOV)]
// 수정일순: 11월(1, 3) 다음 10월(2)
const BY_MODIFIED = [BY_CAPTURED[0], BY_CAPTURED[2], BY_CAPTURED[1]]

const headers = () => screen.queryAllByText(/^\d{4}년 \d{1,2}월$/).map((e) => e.textContent)

describe("Gallery month headers while the sort changes", () => {
  it("T-ui-gsort-01 [Business rule] previous data keeps its own grouping until the new sort's data arrives: no duplicate key, no stale header", async () => {
    // Arrange
    stubLayout({ width: 1000, height: 600 })
    const errors = vi.spyOn(console, "error").mockImplementation(() => undefined)
    const api = createMockApi({ count: 10 })
    const mocks = mockFixedResults(api, (req) => (req.sort.field === "modifiedAt" ? BY_MODIFIED : BY_CAPTURED))
    // 새 정렬의 색인 응답을 붙잡아 둔다 — 도착하기 전까지는 이전 색인(촬영일 묶음)을 그대로 보여 준다
    const realIndex = mocks.index.getMockImplementation() as (req: Parameters<typeof api.searchIndex>[0]) => Promise<SearchIndex>
    let release: () => void = () => undefined
    mocks.index.mockImplementation((req) =>
      req.sort.field === "modifiedAt" ? new Promise<SearchIndex>((resolve) => (release = () => resolve(realIndex(req)))) : realIndex(req)
    )
    renderApp(api)
    await screen.findAllByRole("option")
    expect(headers()).toEqual(["2026년 11월"])
    // Act: 정렬을 수정일로 — 새 결과가 오기 전까지 이전(촬영일순) 데이터가 보인다
    act(() => useSearchStore.getState().setSort({ field: "modifiedAt", direction: "desc" }))
    await waitFor(() => expect(screen.getByRole("listbox")).toHaveAttribute("aria-busy", "true"))
    // Assert: 이전 데이터는 촬영일 기준 그대로 한 그룹, 중복 키 경고 없음
    expect(headers()).toEqual(["2026년 11월"])
    // Act: 새 데이터 도착
    await act(async () => release())
    // Assert: 수정일 기준 두 그룹, 이전 머리글이 남지 않는다
    await waitFor(() => expect(headers()).toEqual(["2026년 11월", "2026년 10월"]))
    const duplicate = errors.mock.calls.some((c) => String(c[0]).includes("same key"))
    expect(duplicate).toBe(false)
  })
})

describe("Gallery grid top padding", () => {
  // 가상화 행(절대 위치 + translateY) 중 맨 위 행
  const firstRowOffset = () => screen.getByRole("listbox").querySelector<HTMLElement>("[style*=translateY]")?.style.transform

  it("T-ui-gpad-01 [Boundary] without month groups the first row starts 12px down; with groups the header row starts at 0", async () => {
    // Arrange: 관련도 정렬 = 그룹 없음
    stubLayout({ width: 1000, height: 600 })
    const api = createMockApi({ count: 10 })
    mockFixedResults(api, BY_CAPTURED)
    renderApp(api)
    await screen.findAllByRole("option")
    // 촬영일순(기본) — 머리글이 첫 행
    expect(firstRowOffset()).toBe("translateY(0px)")
    // Act
    act(() => useSearchStore.getState().setSort({ field: "relevance", direction: "desc" }))
    // Assert
    await waitFor(() => expect(firstRowOffset()).toBe("translateY(12px)"))
    expect(screen.queryAllByText(/^\d{4}년 \d{1,2}월$/)).toHaveLength(0)
  })
})
