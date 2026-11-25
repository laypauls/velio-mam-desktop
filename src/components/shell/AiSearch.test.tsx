// Categories: Happy (T-ui-ai-search-01, T-ui-ai-search-02), Business rule (T-ui-ai-search-03)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import { useSearchStore, useSelectionStore } from "@/stores"
import { findAsset } from "@/test/ai-helpers"
import { renderApp, stubLayout } from "@/test/layout"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("semantic search and similar media (P3)", () => {
  it("T-ui-ai-search-01 [Happy] semantic mode debounces typing into one hybrid request and shows the degraded notice when models are missing", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500 })
    const searchSpy = vi.spyOn(api, "searchAssets")
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")

    // Act: 의미 모드로 전환 후 입력
    await user.click(screen.getByRole("button", { name: "의미" }))
    await user.type(screen.getByRole("combobox", { name: "검색" }), "busan")

    // Assert: 입력이 끝난 뒤 hybrid 요청 1회, degraded 안내
    expect(await screen.findByText(/키워드로 검색함/, undefined, { timeout: 4000 })).toBeInTheDocument()
    const hybrid = searchSpy.mock.calls.filter(([req]) => req.mode === "hybrid")
    expect(hybrid).toHaveLength(1)
    expect(hybrid[0][0].query).toBe("busan")
    expect(searchSpy.mock.calls.filter(([req]) => req.mode === undefined && req.query !== "")).toHaveLength(0)
    expect(await screen.findByText("의미 검색에는 로컬 모델이 필요합니다.")).toBeInTheDocument()

    // Act 2: 키워드 모드로 돌아가면 안내가 사라진다
    await user.click(screen.getByRole("button", { name: "의미" }))
    // Assert 2
    await waitFor(() => expect(screen.queryByText(/키워드로 검색함/)).not.toBeInTheDocument())
  })

  it("T-ui-ai-search-03 [Business rule] semantic mode sends relevance sort, disables the select with an explanation, and restores the user sort when leaving", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500 })
    const searchSpy = vi.spyOn(api, "searchAssets")
    const user = userEvent.setup()
    renderApp(api)
    await screen.findAllByRole("option")
    act(() => useSearchStore.getState().setSort({ field: "name", direction: "asc" }))
    const sortSelect = screen.getByRole("combobox", { name: "정렬 기준" })
    expect(sortSelect).toBeEnabled()

    // Act: 의미 모드 + 질의
    await user.click(screen.getByRole("button", { name: "의미" }))
    await user.type(screen.getByRole("combobox", { name: "검색" }), "busan")
    await screen.findByText(/키워드로 검색함/, undefined, { timeout: 4000 })

    // Assert: 요청은 관련도순, 선택기는 비활성 + 관련도 표시 + 설명, 사용자 정렬은 스토어에 보존
    const hybrid = searchSpy.mock.calls.filter(([req]) => req.mode === "hybrid")
    expect(hybrid.length).toBeGreaterThan(0)
    for (const [req] of hybrid) expect(req.sort).toEqual({ field: "relevance", direction: "desc" })
    expect(sortSelect).toBeDisabled()
    expect(sortSelect).toHaveTextContent("관련도")
    expect(sortSelect).toHaveAccessibleDescription("의미 검색은 관련도순으로만 정렬됩니다.")
    expect(useSearchStore.getState().sort).toEqual({ field: "name", direction: "asc" })

    // Act 2: 키워드 모드로 복귀
    searchSpy.mockClear()
    await user.click(screen.getByRole("button", { name: "의미" }))

    // Assert 2: 사용자가 고른 정렬이 요청에 복원된다
    await waitFor(() => expect(searchSpy).toHaveBeenCalled())
    expect(searchSpy.mock.calls[searchSpy.mock.calls.length - 1]?.[0].sort).toEqual({ field: "name", direction: "asc" })
    expect(screen.getByRole("combobox", { name: "정렬 기준" })).toBeEnabled()
  })

  it("T-ui-ai-search-02[Happy] clicking a similar-media result selects that asset", async () => {
    // Arrange
    stubLayout()
    const api = createMockApi({ count: 500, ai: { clipReady: true, e5Ready: true } })
    const image = await findAsset(api, "image")
    // 유사 탭은 의미 검색이 켜져 있고 기준 자산의 임베딩이 있을 때만 결과를 조회한다
    await api.updateAiSettings({ semanticSearch: { enabled: true } })
    vi.spyOn(api, "getAssetAi").mockResolvedValue({
      analysis: null,
      transcript: null,
      status: [{ kind: "embedding", state: "done", errorCode: null, updatedAt: null }],
    })
    const similar = (await api.findSimilar({ assetId: image.assetId, limit: 24, cursor: null })).items
    const target = similar[0]
    const user = userEvent.setup()
    renderApp(api)
    act(() => useSelectionStore.getState().selectOnly(image.assetId))
    const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })

    // Act
    await user.click(await within(panel).findByRole("tab", { name: "유사" }))
    const list = await within(panel).findByRole("list", { name: "유사 미디어" })
    const pct = Math.round(target.similarity * 100)
    await user.click(within(list).getByRole("button", { name: `${target.fileName} (유사도 ${pct}%)` }))

    // Assert
    expect(useSelectionStore.getState().activeId).toBe(target.assetId)
    expect(useSelectionStore.getState().ids.has(target.assetId)).toBe(true)
    // 선택한 자산 기준으로 목록이 다시 조회된다 (자기 자신은 제외)
    await waitFor(() => {
      const names = within(screen.getByRole("list", { name: "유사 미디어" })).getAllByRole("button").map((b) => b.getAttribute("aria-label"))
      expect(names.some((n) => n?.startsWith(`${target.fileName} `))).toBe(false)
    })
  })
})
