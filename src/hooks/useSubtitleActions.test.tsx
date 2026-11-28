// Categories: Happy (T-ui-subtitle-hook-03) / Business rule (T-ui-subtitle-hook-01 switch off, 02 model missing, 05 force) / External (T-ui-subtitle-hook-04 IPC rejection)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, waitFor } from "@testing-library/react"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AiSettings } from "@/lib/ipc-contract-ai"
import { resetAllStores, useProtectStore } from "@/stores"
import { subtitleBlock, useSubtitleActions } from "./useSubtitleActions"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), warning: vi.fn() } }))

beforeEach(() => resetAllStores())
afterEach(() => {
  vi.restoreAllMocks()
  vi.clearAllMocks()
})

/** 목 API: 설정의 자막 스위치·모델을 지정한다. 목 기본값은 base 준비됨 / small 없음 */
function setup(localStt: AiSettings["localStt"]) {
  const api: UiApi = createMockApi({ count: 10, ai: { consent: true } })
  const base = api.getAiSettings.bind(api)
  vi.spyOn(api, "getAiSettings").mockImplementation(async () => ({ ...(await base()), localStt }))
  const analyze = vi.spyOn(api, "analyzeAssets").mockResolvedValue({ queued: 3, skipped: 1 })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <ApiProvider api={api}>
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>{children}</QueryClientProvider>
    </ApiProvider>
  )
  const hook = renderHook(() => useSubtitleActions(), { wrapper })
  return { analyze, hook }
}

describe("useSubtitleActions", () => {
  it("T-ui-subtitle-hook-01 [Business rule] subtitles switched off → no IPC, guidance toast with a settings button", async () => {
    // Arrange
    const { analyze, hook } = setup({ enabled: false, model: "base" })
    await waitFor(() => expect(hook.result.current.block).toBe("disabled"))
    // Act
    await act(async () => void (await hook.result.current.generate([1, 2])))
    // Assert
    expect(analyze).not.toHaveBeenCalled()
    expect(hook.result.current.available).toBe(false)
    const [message, options] = vi.mocked(toast.warning).mock.calls[0]
    expect(message).toContain("자막이 꺼져 있습니다")
    const action = (options as unknown as { action: { onClick: (e?: unknown) => void } }).action
    action.onClick()
    expect(useProtectStore.getState().settingsOpen).toBe(true)
    expect(useProtectStore.getState().settingsSection).toBe("ai-local")
  })

  it("T-ui-subtitle-hook-02 [Business rule] selected whisper model not installed → no IPC, toast with a settings button", async () => {
    // Arrange
    const { analyze, hook } = setup({ enabled: true, model: "small" })
    await waitFor(() => expect(hook.result.current.block).toBe("modelMissing"))
    // Act
    await act(async () => void (await hook.result.current.generate([1])))
    // Assert
    expect(analyze).not.toHaveBeenCalled()
    expect(vi.mocked(toast.warning).mock.calls[0][0]).toContain("자막 모델이 설치되어 있지 않습니다")
    expect(hook.result.current.reasonText).toContain("자막 모델")
  })

  it("T-ui-subtitle-hook-03 [Happy] ready → analyzeAssets(ids, ['stt'], false) and a subtitle-worded result toast", async () => {
    // Arrange
    const { analyze, hook } = setup({ enabled: true, model: "base" })
    await waitFor(() => expect(hook.result.current.available).toBe(true))
    await waitFor(() => expect(hook.result.current.block).toBeNull())
    // Act
    await act(async () => void (await hook.result.current.generate([1, 2, 3])))
    // Assert
    expect(analyze).toHaveBeenCalledWith([1, 2, 3], ["stt"], false)
    expect(toast.success).toHaveBeenCalledWith("자막 생성 대기 3개", { description: expect.any(String) })
    expect(toast.warning).not.toHaveBeenCalled()
  })

  it("T-ui-subtitle-hook-04 [External] rejected IPC → error toast only (no success toast)", async () => {
    // Arrange
    const { analyze, hook } = setup({ enabled: true, model: "base" })
    await waitFor(() => expect(hook.result.current.available).toBe(true))
    analyze.mockRejectedValueOnce({ code: "internal", message: "boom" })
    // Act
    await act(async () => void (await hook.result.current.generate([1])))
    // Assert
    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(toast.success).not.toHaveBeenCalled()
  })

  it("T-ui-subtitle-hook-05 [Business rule] force=true is passed through for regeneration; nothing queued shows the info toast", async () => {
    // Arrange
    const { analyze, hook } = setup({ enabled: true, model: "base" })
    await waitFor(() => expect(hook.result.current.available).toBe(true))
    analyze.mockResolvedValueOnce({ queued: 0, skipped: 0 })
    // Act
    await act(async () => void (await hook.result.current.generate([9], true)))
    // Assert
    expect(analyze).toHaveBeenCalledWith([9], ["stt"], true)
    expect(toast.info).toHaveBeenCalledWith("새로 자막을 만들 항목이 없습니다.", expect.anything())
  })
})

describe("subtitleBlock", () => {
  it("T-ui-subtitle-hook-06 [Boundary] settings or models not loaded yet → not blocked (never assume 'missing' before the list arrives)", () => {
    // Arrange / Act / Assert
    expect(subtitleBlock(undefined, undefined)).toBeNull()
  })
})
