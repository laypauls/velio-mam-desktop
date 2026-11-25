// Categories: Happy (T-ui-ai-event-01), External (T-ui-ai-event-02 - no events, polling fallback), Boundary (T-ui-ai-event-03 - poll interval)
import { act, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { analysisPollInterval, EVENT_FRESH_MS, noteAnalysisEvent, POLL_FAST_MS, POLL_SLOW_MS, resetAnalysisEventClock } from "@/hooks/useAnalysisEvents"
import { createMockApi } from "@/lib/api/mock"
import { sampleFor } from "@/lib/api/mock-ai-samples"
import { useSelectionStore } from "@/stores"
import { findAsset } from "@/test/ai-helpers"
import { renderApp, stubLayout } from "@/test/layout"

beforeEach(() => resetAnalysisEventClock())
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

/** 수동으로 진행시키는 mock 타이머 - step() 한 번에 대기 중인 전이 하나를 실행한다. */
function manualClock() {
  const queue: Array<() => void> = []
  return {
    schedule: (fn: () => void) => void queue.push(fn),
    step: () => {
      const next = queue.splice(0, 1)
      act(() => next.forEach((fn) => fn()))
    },
  }
}

async function openAiStatus(user: ReturnType<typeof userEvent.setup>, assetId: number) {
  act(() => useSelectionStore.getState().selectOnly(assetId))
  const panel = screen.getByRole("complementary", { name: "상세 정보 패널" })
  await user.click(await within(panel).findByRole("tab", { name: "AI" }))
  return panel
}

describe("analysis events (P3)", () => {
  it("T-ui-ai-event-01 [Happy] an event updates the status badge without polling, and a done step refetches the detail", async () => {
    // Arrange: describe 한 단계만 분석 (running -> done 전이를 수동으로 진행)
    stubLayout()
    const clock = manualClock()
    const api = createMockApi({ count: 500, ai: { consent: true, schedule: clock.schedule } })
    const image = await findAsset(api, "image")
    await api.setApiKey("claude", "sk-test", false)
    const getSpy = vi.spyOn(api, "getAssetAi")
    const user = userEvent.setup()
    renderApp(api)
    const panel = await openAiStatus(user, image.assetId)
    await within(panel).findByText("아직 분석 결과가 없습니다")
    await act(async () => void (await api.analyzeAssets([image.assetId], ["describe"], false)))
    const queued = await within(panel).findByText(/설명 대기/)
    expect(queued.closest("[role=status]")).toHaveAttribute("aria-busy", "true")
    const callsBefore = getSpy.mock.calls.length

    // Act 1: running 전이 (완료 아님)
    clock.step()
    // Assert 1: 재조회 없이 이벤트만으로 배지가 바뀐다
    expect(await within(panel).findByText(/설명 실행 중/)).toBeInTheDocument()
    expect(getSpy.mock.calls.length).toBe(callsBefore)

    // Act 2: done 전이
    clock.step()
    // Assert 2: 상세를 다시 읽어 결과(설명)가 나타난다
    expect(await within(panel).findByText(sampleFor(image.assetId).description)).toBeInTheDocument()
    expect(getSpy.mock.calls.length).toBeGreaterThan(callsBefore)
    await waitFor(() => expect(within(panel).queryByRole("status")).not.toBeInTheDocument())
  })

  it("T-ui-ai-event-02 [External] with no events the 1s polling fallback still reflects the new status", async () => {
    // Arrange: 이벤트 구독을 끊어 둔다 (대량 등록처럼 이벤트가 없는 경로)
    stubLayout()
    const clock = manualClock()
    const api = createMockApi({ count: 500, ai: { consent: true, schedule: clock.schedule } })
    api.onAnalysisChanged = async () => () => undefined
    const image = await findAsset(api, "image")
    await api.setApiKey("claude", "sk-test", false)
    await api.analyzeAssets([image.assetId], ["describe"], false)
    const user = userEvent.setup()
    renderApp(api)
    const panel = await openAiStatus(user, image.assetId)
    expect(await within(panel).findByText(/설명 대기/)).toBeInTheDocument()

    // Act: 화면은 모르는 채로 서버 쪽 상태만 진행
    clock.step()
    clock.step()

    // Assert: 폴링이 완료 상태를 가져온다
    expect(await within(panel).findByText(sampleFor(image.assetId).description, undefined, { timeout: 4000 })).toBeInTheDocument()
    await waitFor(() => expect(within(panel).queryByText(/설명 대기|설명 실행 중/)).not.toBeInTheDocument())
  })

  it("T-ui-ai-event-03 [Boundary] polls every 1s only while busy without recent events, otherwise every 10s", () => {
    // Arrange
    const t0 = 1_000_000

    // Act / Assert
    expect(analysisPollInterval(true, t0)).toBe(POLL_FAST_MS) // 이벤트를 받은 적 없음
    noteAnalysisEvent(t0)
    expect(analysisPollInterval(true, t0 + EVENT_FRESH_MS - 1)).toBe(POLL_SLOW_MS) // 이벤트 정상
    expect(analysisPollInterval(true, t0 + EVENT_FRESH_MS)).toBe(POLL_FAST_MS) // 5초 무소식 -> 폴링 복귀
    expect(analysisPollInterval(false, t0 + EVENT_FRESH_MS)).toBe(POLL_SLOW_MS) // 대기 단계 없음
  })
})
