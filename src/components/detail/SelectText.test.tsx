// Categories: Happy (T-ui-select-01..08) / Business rule (T-ui-select-09 chrome stays non-selectable)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen } from "@testing-library/react"
import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { AppErrorBoundary } from "@/components/shell/AppErrorBoundary"
import { MarkdownDocument } from "@/components/markdown/MarkdownDocument"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetDetail } from "@/lib/ipc-contract"
import type { AiAnalysis, Transcript } from "@/lib/ipc-contract-ai"
import { resetAllStores, useViewStore } from "@/stores"
import { AssetInfo } from "./AssetInfo"
import { AnalysisResult } from "./ai/AnalysisResult"
import { TranscriptSection } from "./ai/TranscriptSection"

beforeEach(() => resetAllStores())
afterEach(() => vi.restoreAllMocks())

const css = readFileSync(resolve(process.cwd(), "src/index.css"), "utf8").replace(/\r\n/g, "\n")

describe("global user-select rules (index.css)", () => {
  it("T-ui-select-01 [Happy] body is user-select none inside @layer base", () => {
    const base = css.slice(css.indexOf("@layer base"))
    expect(base).toMatch(/\n\s*body\s*\{[^}]*user-select:\s*none;[^}]*\}/)
  })

  it("T-ui-select-02 [Happy] input, textarea and contenteditable re-enable text selection", () => {
    expect(css).toMatch(
      /input,\s*textarea,\s*\[contenteditable\]:not\(\[contenteditable="false"\]\)\s*\{\s*user-select:\s*text;\s*\}/
    )
  })
})

async function renderInfo() {
  const api = createMockApi({ count: 10 })
  const base = await api.getAssetDetail(1)
  const detail: AssetDetail = {
    ...base,
    status: "active",
    availability: "online",
    tags: [],
    collections: [],
    duplicates: [],
    previousVersions: [],
    newerVersion: null,
    codec: null,
    audioCodec: null,
    fps: null,
    bitrate: null,
    camera: null,
    volumeLabel: null,
    extra: { shutter: "1/250" },
    title: null,
  }
  useViewStore.getState().setDetailMetaOpen(true)
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ApiProvider api={api}>
        <TooltipProvider>
          <AssetInfo data={detail} />
        </TooltipProvider>
      </ApiProvider>
    </QueryClientProvider>
  )
  return detail
}

describe("detail panel select-text", () => {
  it("T-ui-select-03 [Happy] file name heading and path line are selectable", async () => {
    // Arrange / Act
    const detail = await renderInfo()
    // Assert
    expect(screen.getByRole("heading", { level: 2, name: detail.fileName })).toHaveClass("select-text")
    expect(document.querySelector(`[title="${CSS.escape(detail.path)}"]`)).toHaveClass("select-text")
  })

  it("T-ui-select-04 [Happy] metadata values (dd) are selectable", async () => {
    // Arrange / Act
    await renderInfo()
    // Assert
    const value = await screen.findByText("1/250")
    expect(value.tagName).toBe("DD")
    expect(value).toHaveClass("select-text")
  })

  it("T-ui-select-09 [Business rule] section headings and labels are not selectable", async () => {
    // Arrange / Act
    await renderInfo()
    // Assert
    expect(screen.getByRole("heading", { level: 3, name: "태그" })).not.toHaveClass("select-text")
    expect(screen.getByText("shutter")).not.toHaveClass("select-text")
  })
})

describe("AI tab select-text", () => {
  it("T-ui-select-05 [Happy] analysis description, OCR, summary and keywords are selectable; headings are not", () => {
    // Arrange
    const analysis: AiAnalysis = {
      description: "바다 위의 배",
      keywords: ["바다"],
      ocrText: "OCR 본문",
      summary: "요약 본문",
      language: "ko",
      provider: "local",
      model: "m",
      analyzedAt: 0,
      edited: false,
    }
    // Act
    render(
      <QueryClientProvider client={new QueryClient()}>
        <ApiProvider api={createMockApi({ count: 1 })}>
          <AnalysisResult assetId={1} analysis={analysis} />
        </ApiProvider>
      </QueryClientProvider>
    )
    // Assert
    for (const text of ["바다 위의 배", "OCR 본문", "요약 본문"]) expect(screen.getByText(text)).toHaveClass("select-text")
    expect(screen.getByText("바다")).toHaveClass("select-text")
    expect(screen.getByRole("heading", { level: 3, name: "OCR" })).not.toHaveClass("select-text")
  })

  it("T-ui-select-06 [Happy] STT segment text is selectable", () => {
    // Arrange
    const transcript: Transcript = {
      language: "ko",
      text: "안녕하세요",
      segments: [{ startMs: 0, endMs: 1000, text: "안녕하세요" }],
      engine: "e",
      createdAt: 0,
    }
    // Act
    render(<TranscriptSection assetId={1} transcript={transcript} />)
    // Assert
    expect(screen.getByText("안녕하세요")).toHaveClass("select-text")
  })
})

describe("document and error text select-text", () => {
  it("T-ui-select-07 [Happy] markdown body is selectable", () => {
    // Arrange / Act
    const { container } = render(<MarkdownDocument source={"# 제목\n\n본문"} />)
    // Assert
    expect(container.firstElementChild).toHaveClass("select-text")
  })

  it("T-ui-select-08 [Happy] error boundary description is selectable", () => {
    // Arrange
    vi.spyOn(console, "error").mockImplementation(() => undefined)
    const Boom = (): never => {
      throw new Error("boom")
    }
    // Act
    render(
      <AppErrorBoundary>
        <Boom />
      </AppErrorBoundary>
    )
    // Assert
    const alert = screen.getByRole("alert")
    expect(alert.querySelector("p")).toHaveClass("select-text")
    expect(screen.getByRole("heading", { level: 1 })).not.toHaveClass("select-text")
  })
})
