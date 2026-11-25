// Categories: Happy (T-ui-similar-01,04,05) / Business rule (T-ui-similar-02,03,06,07) / Validation (T-ui-similar-08) / External (T-ui-similar-09)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { cmdError } from "@/lib/api/mock-search"
import type { AnalysisState, AssetAiDetail } from "@/lib/ipc-contract-ai"
import { resetAllStores } from "@/stores"
import { findAsset } from "@/test/ai-helpers"
import { SimilarTab } from "./SimilarTab"

afterEach(() => vi.restoreAllMocks())

function ai(state: AnalysisState | null, errorCode: string | null = null): AssetAiDetail {
  return {
    analysis: null,
    transcript: null,
    status: state === null ? [] : [{ kind: "embedding", state, errorCode, updatedAt: null }],
  }
}

async function setup(opts: { clipReady?: boolean; semantic?: boolean; embedding?: AssetAiDetail }) {
  resetAllStores()
  const api: UiApi = createMockApi({ count: 200, ai: { clipReady: opts.clipReady ?? true, e5Ready: true } })
  const image = await findAsset(api, "image")
  if (opts.semantic) await api.updateAiSettings({ semanticSearch: { enabled: true } })
  if (opts.embedding) vi.spyOn(api, "getAssetAi").mockResolvedValue(opts.embedding)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } } })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <SimilarTab assetId={image.assetId} mediaType="image" />
      </QueryClientProvider>
    </ApiProvider>,
  )
  return { api, assetId: image.assetId }
}

describe("SimilarTab 상태 분기", () => {
  it("T-ui-similar-01 [Happy] models missing shows the download notice", async () => {
    // Arrange / Act
    await setup({ clipReady: false })
    // Assert
    expect(await screen.findByText("유사 미디어를 쓰려면 CLIP 모델이 필요합니다")).toBeInTheDocument()
  })

  it("T-ui-similar-02 [Business rule] models ready + semantic off shows the enable notice and does not query similar", async () => {
    // Arrange
    const { api } = await setup({})
    const spy = vi.spyOn(api, "findSimilar")
    // Assert
    expect(await screen.findByText("의미 검색이 꺼져 있습니다")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "의미 검색 켜기" })).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "AI 설정 열기" })).toBeInTheDocument()
    expect(spy).not.toHaveBeenCalled()
  })

  it("T-ui-similar-03 [Business rule] enabled + embedding running shows a busy status", async () => {
    // Arrange / Act
    const { api } = await setup({ semantic: true, embedding: ai("running") })
    const spy = vi.spyOn(api, "findSimilar")
    // Assert
    const title = await screen.findByText("이 미디어의 임베딩을 만드는 중입니다")
    expect(title.closest("[role=status]")).toHaveAttribute("aria-busy", "true")
    expect(spy).not.toHaveBeenCalled()
  })

  it("T-ui-similar-04 [Business rule] enabled + embedding step missing shows the no-embedding notice", async () => {
    // Arrange / Act
    await setup({ semantic: true, embedding: ai(null) })
    // Assert
    expect(await screen.findByText("이 미디어의 임베딩이 아직 없습니다")).toBeInTheDocument()
    expect(screen.getByRole("button", { name: "지금 만들기" })).toBeInTheDocument()
  })

  it("T-ui-similar-05 [Business rule] failed embedding shows the failure reason", async () => {
    // Arrange / Act
    await setup({ semantic: true, embedding: ai("failed", "unsupported") })
    // Assert
    expect(await screen.findByText("임베딩 생성에 실패했습니다: 지원하지 않는 형식입니다.")).toBeInTheDocument()
  })

  it("T-ui-similar-06 [Business rule] embedding done shows the similar list", async () => {
    // Arrange / Act
    await setup({ semantic: true, embedding: ai("done") })
    // Assert
    expect(await screen.findByRole("list", { name: "유사 미디어" })).toBeInTheDocument()
  })

  it("T-ui-similar-07 [Business rule] embedding done + zero results shows the corrected empty text", async () => {
    // Arrange
    resetAllStores()
    const api: UiApi = createMockApi({ count: 200, ai: { clipReady: true, e5Ready: true } })
    await api.updateAiSettings({ semanticSearch: { enabled: true } })
    await api.updateSettings({ similarMinSimilarity: 0 }) // 기준이 있으면 별도 빈 상태(SimilarLimits.test)
    vi.spyOn(api, "getAssetAi").mockResolvedValue(ai("done"))
    vi.spyOn(api, "findSimilar").mockResolvedValue({ items: [], total: 0, nextCursor: null })
    // Act
    render(
      <ApiProvider api={api}>
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
          <SimilarTab assetId={1} mediaType="image" />
        </QueryClientProvider>
      </ApiProvider>,
    )
    // Assert
    expect(await screen.findByText("유사한 미디어를 찾지 못했습니다")).toBeInTheDocument()
    expect(screen.getByText(/다른 미디어의 임베딩이 만들어지면/)).toBeInTheDocument()
    expect(screen.queryByText(/색인되지 않았을 수/)).not.toBeInTheDocument()
  })
})

describe("SimilarTab 동작", () => {
  it("T-ui-similar-08 [Validation] enable button patches semanticSearch and moves past the notice", async () => {
    // Arrange: 모델 준비 + 꺼짐
    const { api } = await setup({})
    const update = vi.spyOn(api, "updateAiSettings")
    const user = userEvent.setup()
    // Act
    await user.click(await screen.findByRole("button", { name: "의미 검색 켜기" }))
    // Assert
    await waitFor(() => expect(update).toHaveBeenCalledWith({ semanticSearch: { enabled: true } }))
    await waitFor(() => expect(screen.queryByText("의미 검색이 꺼져 있습니다")).not.toBeInTheDocument())
  })

  it("T-ui-similar-09 [External] enable failure shows an inline alert and keeps the notice", async () => {
    // Arrange
    const { api } = await setup({})
    vi.spyOn(api, "updateAiSettings").mockRejectedValue(cmdError("invalid_input", "local models not ready"))
    const user = userEvent.setup()
    // Act
    await user.click(await screen.findByRole("button", { name: "의미 검색 켜기" }))
    // Assert
    expect(await screen.findByRole("alert")).toBeInTheDocument()
    expect(screen.getByText("의미 검색이 꺼져 있습니다")).toBeInTheDocument()
  })

  it("T-ui-similar-10 [Happy] create-now queues only the embedding step for this asset", async () => {
    // Arrange
    const { api, assetId } = await setup({ semantic: true, embedding: ai(null) })
    const analyze = vi.spyOn(api, "analyzeAssets")
    const user = userEvent.setup()
    // Act
    await user.click(await screen.findByRole("button", { name: "지금 만들기" }))
    // Assert
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([assetId], ["embedding"], false))
  })

  it("T-ui-similar-11 [External] create-now failure shows an inline alert", async () => {
    // Arrange
    const { api } = await setup({ semantic: true, embedding: ai(null) })
    vi.spyOn(api, "analyzeAssets").mockRejectedValue(cmdError("invalid_input", "x"))
    const user = userEvent.setup()
    // Act
    await user.click(await screen.findByRole("button", { name: "지금 만들기" }))
    // Assert
    expect(await screen.findByRole("alert")).toBeInTheDocument()
  })
})
