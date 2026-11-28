// Categories: Happy (T-ui-subtitle-sec-01 generate, 02 regenerate confirm) / Business rule (T-ui-subtitle-sec-03 cancel, 04 busy disabled)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { Subtitle } from "@/lib/ipc-contract-ai"
import { resetAllStores } from "@/stores"
import { SubtitleSection } from "./SubtitleSection"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

beforeEach(() => resetAllStores())
afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

const subtitle: Subtitle = {
  language: "ko",
  text: "안녕하세요",
  engine: "whisper",
  createdAt: 0,
  segments: [{ startMs: 0, endMs: 1000, text: "안녕하세요" }],
}

function setup(sub: Subtitle | null, busy = false) {
  const api = createMockApi({ count: 10, ai: { consent: true } })
  const analyze = vi.spyOn(api, "analyzeAssets").mockResolvedValue({ queued: 1, skipped: 0 })
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ApiProvider api={api}>
        <TooltipProvider>
          <SubtitleSection assetId={7} subtitle={sub} busy={busy} />
        </TooltipProvider>
      </ApiProvider>
    </QueryClientProvider>
  )
  return { analyze, user: userEvent.setup() }
}

describe("SubtitleSection 생성 버튼", () => {
  it("T-ui-subtitle-sec-01 [Happy] no subtitle → '자막 생성' requests this asset without force", async () => {
    // Arrange
    const { analyze, user } = setup(null)
    const button = screen.getByRole("button", { name: "자막 생성" })
    await waitFor(() => expect(button).toBeEnabled())
    // Act
    await user.click(button)
    // Assert
    expect(screen.getByText("자막이 없습니다.")).toBeInTheDocument()
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([7], ["stt"], false))
  })

  it("T-ui-subtitle-sec-02 [Happy] has subtitle → '다시 생성' asks for overwrite confirmation, confirm requests force=true", async () => {
    // Arrange
    const { analyze, user } = setup(subtitle)
    // Act 1
    await user.click(screen.getByRole("button", { name: "다시 생성" }))
    const dialog = await screen.findByRole("alertdialog", { name: "자막을 다시 생성할까요?" })
    // Assert 1: 확인 전에는 요청하지 않는다
    expect(dialog).toHaveTextContent("기존 자막을 덮어씁니다.")
    expect(analyze).not.toHaveBeenCalled()
    // Act 2
    await user.click(within(dialog).getByRole("button", { name: "다시 생성" }))
    // Assert 2
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([7], ["stt"], true))
  })

  it("T-ui-subtitle-sec-03 [Business rule] cancelling the overwrite confirmation sends nothing", async () => {
    // Arrange
    const { analyze, user } = setup(subtitle)
    await user.click(screen.getByRole("button", { name: "다시 생성" }))
    await screen.findByRole("alertdialog")
    // Act
    await user.click(screen.getByRole("button", { name: "취소" }))
    // Assert
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull())
    expect(analyze).not.toHaveBeenCalled()
  })

  it.each([false, true])("T-ui-subtitle-sec-04 [Business rule] while the subtitle step is queued/running (hasSubtitle=%s) the button is disabled", (has) => {
    // Arrange / Act
    const { analyze } = setup(has ? subtitle : null, true)
    // Assert
    expect(screen.getByRole("button", { name: has ? "다시 생성" : "자막 생성" })).toBeDisabled()
    expect(analyze).not.toHaveBeenCalled()
  })
})
