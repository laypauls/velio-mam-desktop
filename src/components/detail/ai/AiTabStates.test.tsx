// Categories: Happy (T-ui-ai-state-01,02,05,06) / Business rule (T-ui-ai-state-03,04,07) / Boundary (T-ui-ai-state-08..11)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AiAnalysis, AnalysisStatus, AssetAiDetail, Transcript } from "@/lib/ipc-contract-ai"
import { resetAllStores } from "@/stores"
import { AiTab } from "./AiTab"

beforeEach(() => resetAllStores())
afterEach(() => vi.restoreAllMocks())

const analysis = (over: Partial<AiAnalysis> = {}): AiAnalysis => ({
  description: "해안도로를 따라 이동하는 장면",
  keywords: ["항공 촬영", "해안도로"],
  ocrText: "",
  summary: "",
  language: "ko",
  provider: "claude",
  model: "sonnet",
  analyzedAt: Date.UTC(2026, 10, 3, 7, 12),
  edited: false,
  ...over,
})

const st = (kind: AnalysisStatus["kind"], state: AnalysisStatus["state"], errorCode: string | null = null): AnalysisStatus => ({
  kind,
  state,
  errorCode,
  updatedAt: null,
})

const transcript = (n: number): Transcript => ({
  language: "ko",
  text: "",
  engine: "whisper",
  createdAt: 0,
  segments: Array.from({ length: n }, (_, i) => ({ startMs: i * 1000, endMs: i * 1000 + 900, text: `구간 ${i + 1}` })),
})

function setup(detail: Partial<AssetAiDetail>, mediaType: "image" | "video" = "image") {
  const api: UiApi = createMockApi({ count: 10, ai: { consent: true } })
  const full: AssetAiDetail = { analysis: null, transcript: null, status: [], ...detail }
  vi.spyOn(api, "getAssetAi").mockResolvedValue(full)
  const analyze = vi.spyOn(api, "analyzeAssets").mockResolvedValue({ queued: 1, skipped: 0 })
  const retry = vi.spyOn(api, "retryFailedAnalysis").mockResolvedValue({ queued: 1, skipped: 0 })
  render(
    <ApiProvider api={api}>
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <AiTab assetId={7} mediaType={mediaType} />
      </QueryClientProvider>
    </ApiProvider>,
  )
  return { analyze, retry }
}

describe("AiTab 4상태", () => {
  it("T-ui-ai-state-01 [Happy] done: one muted engine line, no per-kind badge list, no banners", async () => {
    // Arrange / Act
    setup({ analysis: analysis(), status: [st("describe", "done"), st("keywords", "done"), st("embedding", "done")] })
    // Assert
    expect(await screen.findByText("해안도로를 따라 이동하는 장면")).toBeInTheDocument()
    expect(screen.getByText(/^claude · sonnet · /)).toBeInTheDocument()
    expect(screen.queryByRole("status")).not.toBeInTheDocument()
    expect(screen.queryByRole("alert")).not.toBeInTheDocument()
    expect(screen.queryByRole("list", { name: "분석 상태" })).not.toBeInTheDocument()
  })

  it("T-ui-ai-state-02 [Happy] running: busy banner with done/total and waiting kinds, actions disabled, engine line hidden", async () => {
    // Arrange / Act: 임베딩은 요약에서 제외 (D6) — 전체 3 = 설명 완료 + OCR 실행 + 요약 대기
    setup({
      analysis: analysis(),
      status: [st("describe", "done"), st("ocr", "running"), st("summary", "queued"), st("embedding", "queued")],
    })
    // Assert
    const banner = (await screen.findByText(/분석 중 · 1\/3/)).closest("[role=status]")!
    expect(banner).toHaveAttribute("aria-busy", "true")
    expect(banner).toHaveTextContent("분석 중 · 1/3")
    expect(banner).toHaveTextContent("OCR 실행 중 · 요약 대기")
    expect(banner).not.toHaveTextContent("임베딩")
    expect(screen.getByRole("button", { name: "재분석" })).toBeDisabled()
    expect(screen.queryByText(/^claude · sonnet/)).not.toBeInTheDocument()
  })

  it("T-ui-ai-state-03 [Business rule] partially failed: banner lists kinds and reason; retry calls retryFailedAnalysis for this asset only", async () => {
    // Arrange
    const { retry } = setup({
      analysis: analysis(),
      status: [st("describe", "done"), st("ocr", "failed", "rate_limited"), st("summary", "failed", "rate_limited")],
    })
    const user = userEvent.setup()
    // Act
    const alert = await screen.findByRole("alert")
    expect(alert).toHaveTextContent("OCR · 요약 분석 실패")
    expect(alert).toHaveTextContent("요청 한도를 초과했습니다.")
    await user.click(within(alert).getByRole("button", { name: "실패 항목 다시 시도" }))
    // Assert
    await waitFor(() => expect(retry).toHaveBeenCalledWith([7]))
    expect(screen.queryByText(/^claude · sonnet/)).not.toBeInTheDocument()
  })

  it("T-ui-ai-state-04 [Business rule] a failed embedding appears in the failure banner even though it is excluded elsewhere", async () => {
    // Arrange / Act
    setup({ analysis: analysis(), status: [st("describe", "done"), st("embedding", "failed", "unsupported")] })
    // Assert
    const alert = await screen.findByRole("alert")
    expect(alert).toHaveTextContent("임베딩 분석 실패")
    expect(alert).toHaveTextContent("지원하지 않는 형식입니다.")
  })

  it("T-ui-ai-state-05 [Happy] none: empty state with a single 분석 button that calls analyze(force=false)", async () => {
    // Arrange
    const { analyze } = setup({})
    const user = userEvent.setup()
    // Act
    await screen.findByText("아직 분석 결과가 없습니다")
    expect(screen.queryByRole("button", { name: "재분석" })).not.toBeInTheDocument()
    await user.click(screen.getByRole("button", { name: "분석" }))
    // Assert
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([7], null, false))
  })
})

describe("AiTab 결과·버튼", () => {
  it("T-ui-ai-state-06 [Happy] split button: 재분석 forces, the dropdown item '빈 항목만 분석' calls analyze with force=false", async () => {
    // Arrange
    const { analyze } = setup({ analysis: analysis(), status: [st("describe", "done")] })
    const user = userEvent.setup()
    await screen.findByText("해안도로를 따라 이동하는 장면")
    // Act 1
    await user.click(screen.getByRole("button", { name: "재분석" }))
    // Assert 1
    await waitFor(() => expect(analyze).toHaveBeenLastCalledWith([7], null, true))
    // Act 2
    await user.click(screen.getByRole("button", { name: "다른 분석 방식" }))
    await user.click(await screen.findByRole("menuitem", { name: "빈 항목만 분석" }))
    // Assert 2
    await waitFor(() => expect(analyze).toHaveBeenLastCalledWith([7], null, false))
  })

  it("T-ui-ai-state-07 [Business rule] edited result: 재분석 asks for confirmation first; cancel does not call analyze", async () => {
    // Arrange
    const { analyze } = setup({ analysis: analysis({ edited: true }), status: [st("describe", "done")] })
    const user = userEvent.setup()
    await screen.findByText("편집됨")
    // Act
    await user.click(screen.getByRole("button", { name: "재분석" }))
    const dialog = await screen.findByRole("alertdialog", { name: "재분석할까요?" })
    await user.click(within(dialog).getByRole("button", { name: "취소" }))
    // Assert
    expect(analyze).not.toHaveBeenCalled()
  })

  it("T-ui-ai-state-08 [Boundary] empty OCR and summary are hidden (no em dash); keywords are outline chips; edit is an icon button", async () => {
    // Arrange / Act
    setup({ analysis: analysis(), status: [st("describe", "done")] })
    await screen.findByText("해안도로를 따라 이동하는 장면")
    // Assert
    expect(screen.queryByRole("heading", { name: "OCR" })).not.toBeInTheDocument()
    expect(screen.queryByRole("heading", { name: "요약" })).not.toBeInTheDocument()
    expect(screen.queryByText("—")).not.toBeInTheDocument()
    expect(screen.getByRole("button", { name: "편집" })).toBeInTheDocument()
    expect(within(screen.getByRole("list", { name: "키워드" })).getByText("항공 촬영")).toHaveAttribute("data-variant", "outline")
  })

  it("T-ui-ai-state-09 [Boundary] filled OCR and summary are shown", async () => {
    // Arrange / Act
    setup({ analysis: analysis({ ocrText: "STOP", summary: "요약문" }), status: [st("describe", "done")] })
    // Assert
    expect(await screen.findByRole("heading", { name: "OCR" })).toBeInTheDocument()
    expect(screen.getByText("STOP")).toBeInTheDocument()
    expect(screen.getByRole("heading", { name: "요약" })).toBeInTheDocument()
  })

  it("T-ui-ai-state-10 [Boundary] STT shows 3 segments first, '전체 보기' expands to the paged list (50), then 더 보기", async () => {
    // Arrange
    setup({ analysis: analysis(), transcript: transcript(120), status: [st("stt", "done")] }, "video")
    const user = userEvent.setup()
    const list = await screen.findByRole("list", { name: "음성 인식 구간" })
    expect(screen.getByText("음성 인식 · 한국어")).toBeInTheDocument()
    expect(within(list).getAllByRole("button")).toHaveLength(3)
    // Act 1
    await user.click(screen.getByRole("button", { name: "전체 보기 (120구간)" }))
    // Assert 1
    expect(within(list).getAllByRole("button")).toHaveLength(50)
    // Act 2
    await user.click(screen.getByRole("button", { name: "더 보기" }))
    // Assert 2
    expect(within(list).getAllByRole("button")).toHaveLength(100)
  })

  it("T-ui-ai-state-11 [Boundary] STT with 3 or fewer segments has no 전체 보기 button", async () => {
    // Arrange / Act
    setup({ analysis: analysis(), transcript: transcript(3), status: [st("stt", "done")] }, "video")
    // Assert
    const list = await screen.findByRole("list", { name: "음성 인식 구간" })
    expect(within(list).getAllByRole("button")).toHaveLength(3)
    expect(screen.queryByRole("button", { name: /전체 보기/ })).not.toBeInTheDocument()
  })
})
