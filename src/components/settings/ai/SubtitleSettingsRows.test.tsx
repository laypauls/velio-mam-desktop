// Categories: Happy (T-ui-subtitle-set-02/03/06) / Business rule (T-ui-subtitle-set-01 effective value, 04 no consent, 05 gating) / External (T-ui-subtitle-set-08 IPC error) / Boundary (T-ui-subtitle-set-07 nothing queued)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { effectiveAutoSubtitle, type AiSettings } from "@/lib/ipc-contract-ai"
import { SubtitleSettingsRows } from "./SubtitleSettingsRows"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

async function setup(over: Partial<AiSettings> = {}, consent = true) {
  const api = createMockApi({ count: 10, ai: { consent } })
  const settings: AiSettings = { ...(await api.getAiSettings()), ...over }
  vi.spyOn(api, "getAiSettings").mockResolvedValue(settings)
  const patch = vi.fn()
  const bulk = vi.spyOn(api, "generateSubtitlesAll").mockResolvedValue({ queued: 1234, skipped: 0 })
  render(
    <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
      <ApiProvider api={api}>
        <SubtitleSettingsRows ctl={{ settings, patch, commit: async () => undefined }} />
      </ApiProvider>
    </QueryClientProvider>
  )
  return { api, patch, bulk, user: userEvent.setup() }
}

const autoSwitch = () => screen.getByRole("switch", { name: "새 파일 자동 자막 생성" })

describe("effectiveAutoSubtitle", () => {
  it.each([
    [null, true, true, true],
    [null, true, false, false],
    [null, false, true, false],
    [true, false, false, true],
    [false, true, true, false],
  ])("T-ui-subtitle-set-01 [Business rule] autoSubtitleNew=%s autoAnalyzeNew=%s consent=%s → %s", (sub, analyze, given, want) => {
    // Arrange / Act / Assert
    expect(effectiveAutoSubtitle({ autoSubtitleNew: sub, autoAnalyzeNew: analyze, consent: { given, at: null } })).toBe(want)
  })
})

describe("SubtitleSettingsRows", () => {
  it.each([
    [null, "unchecked"],
    [true, "checked"],
    [false, "unchecked"],
  ] as const)("T-ui-subtitle-set-02 [Happy] autoSubtitleNew=%s shows the effective value (%s) when auto analysis is off", async (value, state) => {
    // Arrange / Act
    await setup({ autoSubtitleNew: value, autoAnalyzeNew: false })
    // Assert
    await waitFor(() => expect(autoSwitch()).toBeEnabled())
    expect(autoSwitch()).toHaveAttribute("data-state", state)
  })

  it("T-ui-subtitle-set-03 [Happy] null follows auto analysis + consent, and toggling patches { autoSubtitleNew }", async () => {
    // Arrange
    const { patch, user } = await setup({ autoSubtitleNew: null, autoAnalyzeNew: true })
    await waitFor(() => expect(autoSwitch()).toBeEnabled())
    expect(autoSwitch()).toHaveAttribute("data-state", "checked")
    // Act
    await user.click(autoSwitch())
    // Assert
    expect(patch).toHaveBeenCalledWith({ autoSubtitleNew: false })
  })

  it("T-ui-subtitle-set-04 [Business rule] works without external AI consent (switch enabled, mock accepts the patch)", async () => {
    // Arrange
    const { api, patch, user } = await setup({ autoSubtitleNew: false }, false)
    await waitFor(() => expect(autoSwitch()).toBeEnabled())
    // Act
    await user.click(autoSwitch())
    // Assert
    expect(patch).toHaveBeenCalledWith({ autoSubtitleNew: true })
    await expect(api.updateAiSettings({ autoSubtitleNew: true })).resolves.toMatchObject({ autoSubtitleNew: true })
  })

  it.each([
    ["subtitles off", { localStt: { enabled: false, model: "base" as const } }, "자막을 켜세요."],
    ["model missing", { localStt: { enabled: true, model: "small" as const } }, "로컬 모델에서 자막 모델을 설치하세요."],
  ])("T-ui-subtitle-set-05 [Business rule] %s → both rows disabled with the reason", async (_name, over, reason) => {
    // Arrange / Act
    const { bulk } = await setup(over)
    // Assert
    await waitFor(() => expect(autoSwitch()).toBeDisabled())
    expect(screen.getByRole("button", { name: "자막 없는 항목 생성" })).toBeDisabled()
    expect(screen.getAllByText(reason)).toHaveLength(2)
    expect(bulk).not.toHaveBeenCalled()
  })

  it("T-ui-subtitle-set-06 [Happy] bulk: cancel does not call; confirm calls the IPC once and shows the count toast", async () => {
    // Arrange
    const { bulk, user } = await setup()
    const open = screen.getByRole("button", { name: "자막 없는 항목 생성" })
    await waitFor(() => expect(autoSwitch()).toBeEnabled())
    // Act 1: cancel
    await user.click(open)
    await screen.findByRole("alertdialog", { name: "자막을 일괄 생성할까요?" })
    await user.click(screen.getByRole("button", { name: "취소" }))
    // Assert 1
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull())
    expect(bulk).not.toHaveBeenCalled()
    // Act 2: confirm
    await user.click(open)
    await user.click(await screen.findByRole("button", { name: "생성 시작" }))
    // Assert 2
    await waitFor(() => expect(toast.success).toHaveBeenCalledWith("자막 생성 대기 1,234개"))
    expect(bulk).toHaveBeenCalledTimes(1)
  })

  it("T-ui-subtitle-set-07 [Boundary] bulk with nothing to queue shows the nothing-to-do message", async () => {
    // Arrange
    const { bulk, user } = await setup()
    bulk.mockResolvedValueOnce({ queued: 0, skipped: 0 })
    await waitFor(() => expect(autoSwitch()).toBeEnabled())
    // Act
    await user.click(screen.getByRole("button", { name: "자막 없는 항목 생성" }))
    await user.click(await screen.findByRole("button", { name: "생성 시작" }))
    // Assert
    await waitFor(() => expect(toast.info).toHaveBeenCalledWith("새로 자막을 만들 항목이 없습니다."))
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("T-ui-subtitle-set-08 [External] bulk IPC error shows only an error toast", async () => {
    // Arrange
    const { bulk, user } = await setup()
    bulk.mockRejectedValueOnce({ code: "invalid_input", message: "off" })
    await waitFor(() => expect(autoSwitch()).toBeEnabled())
    // Act
    await user.click(screen.getByRole("button", { name: "자막 없는 항목 생성" }))
    await user.click(await screen.findByRole("button", { name: "생성 시작" }))
    // Assert
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    expect(toast.success).not.toHaveBeenCalled()
  })
})
