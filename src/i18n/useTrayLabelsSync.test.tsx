// Categories: Happy (mount push, re-push on language change) / External failure (rejection swallowed) / Concurrency (unsubscribe on unmount)
import { renderHook, act } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import i18n from "@/i18n"
import { useTrayLabelsSync } from "@/i18n/useTrayLabelsSync"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"

let original: string

beforeEach(async () => {
  original = i18n.language
  await i18n.changeLanguage("en")
})

afterEach(async () => {
  vi.restoreAllMocks()
  await i18n.changeLanguage(original)
})

function setup(rejection?: unknown) {
  const api = createMockApi({ count: 1 })
  const spy = vi.spyOn(api, "setTrayLabels")
  if (rejection !== undefined) spy.mockRejectedValue(rejection)
  const wrapper = ({ children }: { children: ReactNode }) => <ApiProvider api={api}>{children}</ApiProvider>
  return { spy, wrapper }
}

describe("useTrayLabelsSync", () => {
  it("T-ui-tray-01 [Happy] pushes labels in the current language on mount, with the app name interpolated", () => {
    // Arrange
    const { spy, wrapper } = setup()
    // Act
    renderHook(() => useTrayLabelsSync(), { wrapper })
    // Assert
    expect(spy).toHaveBeenCalledTimes(1)
    expect(spy).toHaveBeenCalledWith({
      open: "Open",
      pauseJobs: "Pause jobs",
      resumeJobs: "Resume jobs",
      quit: "Quit",
      aiWaiting: "velio. - Waiting for AI requests to finish…",
    })
  })

  it("T-ui-tray-02 [Happy] pushes again with the new labels after the language changes", async () => {
    // Arrange
    const { spy, wrapper } = setup()
    renderHook(() => useTrayLabelsSync(), { wrapper })
    // Act
    await act(async () => {
      await i18n.changeLanguage("ko")
    })
    // Assert
    expect(spy).toHaveBeenCalledTimes(2)
    expect(spy).toHaveBeenLastCalledWith({
      open: "열기",
      pauseJobs: "작업 일시정지",
      resumeJobs: "작업 재개",
      quit: "종료",
      aiWaiting: "velio. - AI 요청 완료를 기다리는 중…",
    })
  })

  it("T-ui-tray-03 [External] a rejected command is swallowed on mount and on language change", async () => {
    // Arrange
    const unhandled = vi.fn()
    process.on("unhandledRejection", unhandled)
    const { spy, wrapper } = setup({ code: "internal", message: "boom" })
    // Act
    renderHook(() => useTrayLabelsSync(), { wrapper })
    await act(async () => {
      await i18n.changeLanguage("ko")
    })
    await new Promise((r) => setTimeout(r, 0))
    process.off("unhandledRejection", unhandled)
    // Assert
    expect(spy).toHaveBeenCalledTimes(2)
    expect(unhandled).not.toHaveBeenCalled()
  })

  it("T-ui-tray-04 [Concurrency] unsubscribes on unmount — later language changes push nothing", async () => {
    // Arrange
    const { spy, wrapper } = setup()
    const { unmount } = renderHook(() => useTrayLabelsSync(), { wrapper })
    expect(spy).toHaveBeenCalledTimes(1)
    // Act
    unmount()
    await act(async () => {
      await i18n.changeLanguage("ko")
    })
    // Assert
    expect(spy).toHaveBeenCalledTimes(1)
  })
})
