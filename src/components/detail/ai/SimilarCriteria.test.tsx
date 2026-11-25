// Categories: Happy (T-ui-similar-23,24,25) / Validation (T-ui-similar-26) / External (T-ui-similar-27) / Boundary (T-ui-similar-28)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { cmdError } from "@/lib/api/mock-search"
import type { SettingsPatch } from "@/lib/ipc-contract"
import { resetAllStores } from "@/stores"
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

async function setup(settings: SettingsPatch) {
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
  const find = vi.spyOn(api, "findSimilar")
  const update = vi.spyOn(api, "updateSettings")
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })}>
        <SimilarTab assetId={image.assetId} mediaType="image" />
      </QueryClientProvider>
    </ApiProvider>,
  )
  return { api, find, update }
}

describe("SimilarTab 기준 Popover", () => {
  it("T-ui-similar-23 [Happy] chip opens a popover with the slider, max choices and the global-scope note", async () => {
    // Arrange
    await setup({ similarMaxResults: 24, similarMinSimilarity: 0.75 })
    const user = userEvent.setup()
    // Act
    await user.click(await screen.findByRole("button", { name: "75% 이상 · 최대 24개" }))
    // Assert
    const slider = await screen.findByRole("slider", { name: "최소 유사도" })
    expect(slider).toHaveAttribute("aria-valuenow", "75")
    expect(slider).toHaveAttribute("aria-valuemax", "99")
    expect(screen.getByRole("radio", { name: "24" })).toBeChecked()
    expect(screen.getByText("모든 미디어의 유사 탭에 적용됩니다.")).toBeInTheDocument()
  })

  it("T-ui-similar-24 [Happy] releasing the slider saves similarMinSimilarity once and refetches from the first page", async () => {
    // Arrange
    const { update, find } = await setup({ similarMaxResults: 24, similarMinSimilarity: 0.75 })
    const user = userEvent.setup()
    await user.click(await screen.findByRole("button", { name: "75% 이상 · 최대 24개" }))
    const slider = await screen.findByRole("slider", { name: "최소 유사도" })
    await screen.findByRole("list", { name: "유사 미디어" })
    const before = find.mock.calls.length
    update.mockClear()
    // Act: 키보드 한 칸 = commit 한 번
    slider.focus()
    await user.keyboard("{ArrowRight}")
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
    expect(update).toHaveBeenCalledWith({ similarMinSimilarity: 0.76 })
    await waitFor(() => expect(find.mock.calls.length).toBeGreaterThan(before))
    expect(find.mock.calls[find.mock.calls.length - 1][0].cursor).toBeNull()
    expect(await screen.findByRole("button", { name: "76% 이상 · 최대 24개" })).toBeInTheDocument()
  })

  it("T-ui-similar-25 [Happy] choosing a max count saves similarMaxResults; re-pressing the selected one does nothing", async () => {
    // Arrange
    const { update } = await setup({ similarMaxResults: 24, similarMinSimilarity: 0.75 })
    const user = userEvent.setup()
    await user.click(await screen.findByRole("button", { name: "75% 이상 · 최대 24개" }))
    const group = await screen.findByRole("radiogroup", { name: "최대 개수" })
    update.mockClear()
    // Act 1
    await user.click(within(group).getByRole("radio", { name: "48" }))
    // Assert 1
    await waitFor(() => expect(update).toHaveBeenCalledWith({ similarMaxResults: 48 }))
    await screen.findByRole("button", { name: "75% 이상 · 최대 48개" })
    // Act 2: 이미 선택된 값
    update.mockClear()
    await user.click(within(group).getByRole("radio", { name: "48" }))
    // Assert 2
    expect(update).not.toHaveBeenCalled()
  })

  it("T-ui-similar-26 [Boundary] a current max outside the presets (settings screen allows 6..200) is still listed", async () => {
    // Arrange / Act
    await setup({ similarMaxResults: 30, similarMinSimilarity: 0 })
    const user = userEvent.setup()
    await user.click(await screen.findByRole("button", { name: "최대 30개" }))
    // Assert
    expect(await screen.findByRole("radio", { name: "30" })).toBeChecked()
  })

  it("T-ui-similar-27 [External] a failed save shows an inline alert in the popover and keeps the old value", async () => {
    // Arrange
    const { update } = await setup({ similarMaxResults: 24, similarMinSimilarity: 0.75 })
    const user = userEvent.setup()
    await user.click(await screen.findByRole("button", { name: "75% 이상 · 최대 24개" }))
    const group = await screen.findByRole("radiogroup", { name: "최대 개수" })
    update.mockRejectedValueOnce(cmdError("invalid_input", "similarMaxResults must be an integer 6..200"))
    // Act
    await user.click(within(group).getByRole("radio", { name: "12" }))
    // Assert
    expect(await screen.findByRole("alert")).toBeInTheDocument()
    expect(screen.getByRole("radio", { name: "24" })).toBeChecked()
  })

  it("T-ui-similar-28 [Business rule] threshold-empty [기준 변경] opens the same popover, which can lower the threshold and bring results back", async () => {
    // Arrange: 99% 이상은 mock 의 최고 유사도(97%)보다 높아 0건
    const { update } = await setup({ similarMaxResults: 24, similarMinSimilarity: 0.99 })
    const user = userEvent.setup()
    // Act 1
    await user.click(await screen.findByRole("button", { name: "기준 변경" }))
    const slider = await screen.findByRole("slider", { name: "최소 유사도" })
    // Act 2: 99 -> 98
    slider.focus()
    await user.keyboard("{ArrowLeft}")
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ similarMinSimilarity: 0.98 }))
    expect(screen.getByRole("button", { name: "98% 이상 · 최대 24개" })).toBeInTheDocument()
  })
})

describe("SimilarTab 항목 표시", () => {
  it("T-ui-similar-29 [Happy] items carry a name tooltip; only videos with a duration show the duration badge", async () => {
    // Arrange: findSimilar 를 고정 응답(동영상 1·이미지 1)으로 바꾼다
    resetAllStores()
    const api: UiApi = createMockApi({ count: 200, ai: { clipReady: true, e5Ready: true } })
    await api.updateAiSettings({ semanticSearch: { enabled: true } })
    await api.updateSettings({ similarMinSimilarity: 0 })
    const base = await api.getAssetDetail(2)
    vi.spyOn(api, "getAssetAi").mockResolvedValue({
      analysis: null,
      transcript: null,
      status: [{ kind: "embedding", state: "done", errorCode: null, updatedAt: null }],
    })
    vi.spyOn(api, "findSimilar").mockResolvedValue({
      items: [
        { ...base, assetId: 11, fileName: "clip.mp4", title: null, mediaType: "video", durationMs: 75_000, similarity: 0.9 },
        { ...base, assetId: 12, fileName: "pic.jpg", title: null, mediaType: "image", durationMs: null, similarity: 0.8 },
      ],
      total: 2,
      nextCursor: null,
    })
    // Act
    render(
      <ApiProvider api={api}>
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
          <SimilarTab assetId={1} mediaType="image" />
        </QueryClientProvider>
      </ApiProvider>,
    )
    // Assert
    const list = await screen.findByRole("list", { name: "유사 미디어" })
    const [clip, pic] = within(list).getAllByRole("button")
    expect(clip).toHaveAttribute("title", "clip.mp4")
    expect(pic).toHaveAttribute("title", "pic.jpg")
    expect(within(clip).getByText("1:15")).toBeInTheDocument()
    expect(within(pic).queryByText(/:/)).not.toBeInTheDocument()
    expect(screen.getByText("2개 중 2개")).toBeInTheDocument()
  })
})
