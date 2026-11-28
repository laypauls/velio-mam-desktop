// Categories: Happy (T-ui-subtitle-skip-01 reasons, 02 retry) / Business rule (T-ui-subtitle-skip-03 settings action, 04 other states hidden)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { TooltipProvider } from "@/components/ui/tooltip"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AnalysisStatus } from "@/lib/ipc-contract-ai"
import { resetAllStores, useProtectStore } from "@/stores"
import { SubtitleSection } from "./SubtitleSection"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

beforeEach(() => resetAllStores())
afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

const stt = (state: AnalysisStatus["state"], errorCode: string | null = null): AnalysisStatus => ({ kind: "stt", state, errorCode, updatedAt: null })

function setup(status: AnalysisStatus) {
  const api = createMockApi({ count: 10, ai: { consent: true } })
  const analyze = vi.spyOn(api, "analyzeAssets").mockResolvedValue({ queued: 1, skipped: 0 })
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ApiProvider api={api}>
        <TooltipProvider>
          <SubtitleSection assetId={7} subtitle={null} status={status} />
        </TooltipProvider>
      </ApiProvider>
    </QueryClientProvider>
  )
  return { analyze, user: userEvent.setup() }
}

describe("SubtitleSection skip reason", () => {
  it.each([
    ["unsupported", "자막 도구를 찾을 수 없어 자막을 만들지 못했습니다."],
    ["model_missing", "자막 모델이 설치되어 있지 않아"],
    ["disabled", "자막이 꺼져 있어 자막을 만들지 못했습니다."],
    ["asset_inactive", "파일을 사용할 수 없는 상태라"],
    ["something_new", "자막을 만들지 못했습니다."],
  ])("T-ui-subtitle-skip-01 [Happy] skipped/%s shows its reason message", (code, text) => {
    // Arrange / Act
    setup(stt("skipped", code))
    // Assert
    expect(screen.getByRole("alert")).toHaveTextContent(text)
  })

  it("T-ui-subtitle-skip-02 [Happy] 다시 시도 requests the subtitle for this asset with force=false", async () => {
    // Arrange
    const { analyze, user } = setup(stt("skipped", "unsupported"))
    // Act
    await user.click(screen.getByRole("button", { name: "다시 시도" }))
    // Assert
    await waitFor(() => expect(analyze).toHaveBeenCalledWith([7], ["stt"], false))
  })

  it("T-ui-subtitle-skip-03 [Business rule] settings-fixable reasons also offer opening settings; tool-missing does not", async () => {
    // Arrange
    const first = setup(stt("skipped", "model_missing"))
    // Act
    await first.user.click(screen.getByRole("button", { name: "설정 열기" }))
    // Assert
    expect(useProtectStore.getState().settingsSection).toBe("ai-local")
  })

  it("T-ui-subtitle-skip-03b [Business rule] unsupported has no open-settings action", () => {
    // Arrange / Act
    setup(stt("skipped", "unsupported"))
    // Assert
    expect(screen.queryByRole("button", { name: "설정 열기" })).toBeNull()
  })

  it.each(["done", "queued", "running", "none", "failed"] as const)("T-ui-subtitle-skip-04 [Business rule] state %s shows no skip notice", (state) => {
    // Arrange / Act
    setup(stt(state))
    // Assert
    expect(screen.queryByRole("alert")).toBeNull()
    expect(screen.queryByRole("button", { name: "다시 시도" })).toBeNull()
  })
})
