// Categories: Boundary (T-ui-similar-12,13,14,16) / Business rule (T-ui-similar-17,18,19,20,21,22) / Happy (T-ui-similar-15)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, renderHook, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { qk } from "@/hooks/keys"
import { useUpdateSettings } from "@/hooks/useSettings"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { Page, SettingsPatch } from "@/lib/ipc-contract"
import type { SimilarItem } from "@/lib/ipc-contract-ai"
import { resetAllStores, useProtectStore } from "@/stores"
import { findAsset } from "@/test/ai-helpers"
import { SimilarTab } from "./SimilarTab"

// Radix Slider/Popover 가 ResizeObserver 를 요구한다 — 아무것도 호출하지 않는 스텁
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  )
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 임베딩 완료 상태의 유사 탭을 렌더한다. `page` 를 주면 findSimilar 를 그 응답으로 고정한다. */
async function setup(settings: SettingsPatch, page?: (api: UiApi) => Promise<Page<SimilarItem>>) {
  resetAllStores()
  const api: UiApi = createMockApi({ count: 200, ai: { clipReady: true, e5Ready: true } })
  await api.updateAiSettings({ semanticSearch: { enabled: true } })
  await api.updateSettings(settings)
  const image = await findAsset(api, "image")
  vi.spyOn(api, "getAssetAi").mockResolvedValue({
    analysis: null,
    transcript: null,
    status: [{ kind: "embedding", state: "done", errorCode: null, updatedAt: null }],
  })
  const fixed = page ? await page(api) : null
  const find = vi.spyOn(api, "findSimilar")
  if (fixed) find.mockResolvedValue(fixed)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <SimilarTab assetId={image.assetId} mediaType="image" />
      </QueryClientProvider>
    </ApiProvider>,
  )
  return { api, find, queryClient }
}

const itemCount = () => screen.getAllByRole("listitem").length

describe("SimilarTab 개수 상한", () => {
  it("T-ui-similar-12 [Boundary] max below the page size shrinks the first limit and hides 'more' at the cap", async () => {
    // Arrange / Act
    const { find } = await setup({ similarMaxResults: 10, similarMinSimilarity: 0 })
    await screen.findByRole("list", { name: "유사 미디어" })
    // Assert: mock 은 48개와 다음 커서를 주지만 상한(10)에서 멈춘다
    expect(find).toHaveBeenCalledTimes(1)
    expect(find.mock.calls[0][0].limit).toBe(10)
    expect(itemCount()).toBe(10)
    expect(screen.queryByRole("button", { name: "더 보기" })).not.toBeInTheDocument()
  })

  it("T-ui-similar-13 [Boundary] next page limit is the remaining count and 'more' disappears at the cap", async () => {
    // Arrange
    const { find } = await setup({ similarMaxResults: 30, similarMinSimilarity: 0 })
    const user = userEvent.setup()
    // Act
    await user.click(await screen.findByRole("button", { name: "더 보기" }))
    // Assert
    await waitFor(() => expect(itemCount()).toBe(30))
    expect(find.mock.calls.map((c) => c[0].limit)).toEqual([24, 6])
    expect(screen.queryByRole("button", { name: "더 보기" })).not.toBeInTheDocument()
  })

  it("T-ui-similar-14 [Boundary] no 'more' when the backend stops at the threshold (no next cursor)", async () => {
    // Arrange / Act: 하한 75% → mock 은 24개 미만에서 끊고 nextCursor null
    await setup({ similarMaxResults: 100, similarMinSimilarity: 0.75 })
    // Assert
    await screen.findByRole("list", { name: "유사 미디어" })
    expect(itemCount()).toBeLessThan(24)
    expect(screen.queryByRole("button", { name: "더 보기" })).not.toBeInTheDocument()
  })
})

describe("SimilarTab 기준 안내·빈 상태", () => {
  it("T-ui-similar-15 [Happy] header chip shows the threshold and the cap, and the result count uses Page.total", async () => {
    // Arrange
    await setup({ similarMaxResults: 24, similarMinSimilarity: 0.75 })
    // Assert
    expect(await screen.findByRole("button", { name: "75% 이상 · 최대 24개" })).toBeInTheDocument()
    await screen.findByRole("list", { name: "유사 미디어" })
    expect(screen.getByText(new RegExp(`^[\\d,]+개 중 ${itemCount()}개$`))).toBeInTheDocument()
  })

  it("T-ui-similar-16 [Boundary] header omits the threshold when min is 0", async () => {
    // Arrange / Act
    await setup({ similarMaxResults: 12, similarMinSimilarity: 0 })
    // Assert
    expect(await screen.findByRole("button", { name: "최대 12개" })).toBeInTheDocument()
    expect(screen.queryByText(/\d+% 이상/)).not.toBeInTheDocument()
  })

  it("T-ui-similar-17 [Business rule] zero results with a threshold shows the threshold empty state and its button opens the criteria popover (not the settings screen)", async () => {
    // Arrange: 99% 이상은 mock 의 최고 유사도(97%)보다 높아 0건
    await setup({ similarMinSimilarity: 0.99 })
    const user = userEvent.setup()
    // Assert
    expect(await screen.findByText("99% 이상 비슷한 미디어가 없습니다")).toBeInTheDocument()
    expect(screen.queryByText("유사한 미디어를 찾지 못했습니다")).not.toBeInTheDocument()
    // Act
    await user.click(screen.getByRole("button", { name: "기준 변경" }))
    // Assert
    expect(await screen.findByRole("slider", { name: "최소 유사도" })).toBeInTheDocument()
    expect(useProtectStore.getState().settingsOpen).toBe(false)
  })

  it("T-ui-similar-18 [Boundary] zero results without a threshold keeps the plain empty state", async () => {
    // Arrange / Act
    await setup({ similarMinSimilarity: 0 }, async () => ({ items: [], total: 0, nextCursor: null }))
    // Assert
    expect(await screen.findByText("유사한 미디어를 찾지 못했습니다")).toBeInTheDocument()
    expect(screen.queryByRole("button", { name: "기준 변경" })).not.toBeInTheDocument()
  })
})

describe("SimilarTab 설정 반영", () => {
  it("T-ui-similar-19 [Business rule] changed settings refetch from the first page with the new limit", async () => {
    // Arrange
    const { api, find, queryClient } = await setup({ similarMaxResults: 24, similarMinSimilarity: 0.75 })
    await screen.findByRole("list", { name: "유사 미디어" })
    const before = find.mock.calls.length
    // Act
    queryClient.setQueryData(qk.settings, await api.updateSettings({ similarMaxResults: 8, similarMinSimilarity: 0.9 }))
    // Assert
    await waitFor(() => expect(find.mock.calls.length).toBeGreaterThan(before))
    const last = find.mock.calls[find.mock.calls.length - 1][0]
    expect(last.limit).toBe(8)
    expect(last.cursor).toBeNull()
    expect(await screen.findByRole("button", { name: "90% 이상 · 최대 8개" })).toBeInTheDocument()
    await waitFor(() => expect(itemCount()).toBe(8))
  })

  it("T-ui-similar-20 [Business rule] saving the similar settings invalidates similar queries; other fields do not", async () => {
    // Arrange
    const api = createMockApi({ count: 50 })
    const queryClient = new QueryClient()
    const invalidate = vi.spyOn(queryClient, "invalidateQueries")
    const wrapper = ({ children }: { children: ReactNode }) => (
      <ApiProvider api={api}>
        <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
      </ApiProvider>
    )
    const { result } = renderHook(() => useUpdateSettings({ silent: true }), { wrapper })
    // Act 1: 무관한 필드
    await result.current.mutateAsync({ closeToTray: false })
    // Assert 1
    expect(invalidate).not.toHaveBeenCalledWith({ queryKey: qk.similarAll })
    // Act 2
    await result.current.mutateAsync({ similarMinSimilarity: 0.8 })
    // Assert 2
    expect(invalidate).toHaveBeenCalledWith({ queryKey: qk.similarAll })
  })
})

describe("SimilarTab 표시 이름", () => {
  const titled = (api: UiApi): Promise<Page<SimilarItem>> =>
    api.getAssetDetail(2).then((d) => ({ items: [{ ...d, fileName: "a.jpg", title: "내 제목", similarity: 0.9 }], total: 1, nextCursor: null }))

  it("T-ui-similar-21 [Business rule] item aria-label uses the title when title editing is on", async () => {
    await setup({ titleEditEnabled: true, similarMinSimilarity: 0 }, titled)
    expect(await screen.findByRole("button", { name: "내 제목 (유사도 90%)" })).toBeInTheDocument()
  })

  it("T-ui-similar-22 [Business rule] item aria-label falls back to the file name when title editing is off", async () => {
    await setup({ titleEditEnabled: false, similarMinSimilarity: 0 }, titled)
    expect(await screen.findByRole("button", { name: "a.jpg (유사도 90%)" })).toBeInTheDocument()
  })
})
