// Categories: Happy (T-ui-aistate-10, 11) / Business rule (T-ui-aistate-12, 13) / Time (T-ui-aistate-14)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook } from "@testing-library/react"
import type { ReactNode } from "react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { ApiProvider, type UiApi } from "@/lib/api"
import type { AnalysisChange, AnalysisKind, AnalysisState } from "@/lib/ipc-contract-ai"
import { qk } from "./keys"
import { SEARCH_INVALIDATE_MS, useAnalysisEvents } from "./useAnalysisEvents"

type Listener = (changes: AnalysisChange[]) => void

const change = (kind: AnalysisKind, state: AnalysisState, assetId = 1): AnalysisChange => ({
  assetId,
  status: [{ kind, state, errorCode: state === "failed" ? "offline" : null, updatedAt: 1 }],
})

function setup() {
  let listener: Listener = () => {}
  const api = { onAnalysisChanged: async (fn: Listener) => ((listener = fn), () => {}) } as unknown as UiApi
  const qc = new QueryClient()
  const spy = vi.spyOn(qc, "invalidateQueries")
  const wrapper = ({ children }: { children: ReactNode }) => (
    <ApiProvider api={api}>
      <QueryClientProvider client={qc}>{children}</QueryClientProvider>
    </ApiProvider>
  )
  renderHook(() => useAnalysisEvents(), { wrapper })
  const emit = async (...cs: AnalysisChange[]) => {
    await act(async () => listener(cs))
    await act(async () => void vi.advanceTimersByTime(SEARCH_INVALIDATE_MS))
  }
  const searchCalls = () => spy.mock.calls.filter(([f]) => f?.queryKey === qk.search).length
  return { emit, searchCalls }
}

beforeEach(() => vi.useFakeTimers())
afterEach(() => vi.useRealTimers())

describe("analysis events -> gallery refresh", () => {
  it("T-ui-aistate-10 [Happy] stt/ocr/summary reaching done refreshes the gallery search once batched", async () => {
    const { emit, searchCalls } = setup()
    await act(async () => void (await vi.advanceTimersByTimeAsync(0)))
    await emit(change("stt", "done"), change("ocr", "done", 2), change("summary", "done", 3))
    expect(searchCalls()).toBe(1)
  })

  it("T-ui-aistate-11 [Happy] a non-embedding step reaching failed refreshes the gallery search", async () => {
    const { emit, searchCalls } = setup()
    await act(async () => void (await vi.advanceTimersByTimeAsync(0)))
    await emit(change("describe", "failed"))
    expect(searchCalls()).toBe(1)
  })

  it("T-ui-aistate-12 [Business rule] embedding done/failed alone never refreshes the gallery search", async () => {
    const { emit, searchCalls } = setup()
    await act(async () => void (await vi.advanceTimersByTimeAsync(0)))
    await emit(change("embedding", "done"), change("embedding", "failed", 2))
    expect(searchCalls()).toBe(0)
  })

  it("T-ui-aistate-13 [Business rule] progress states (queued/running) do not refresh the gallery search", async () => {
    const { emit, searchCalls } = setup()
    await act(async () => void (await vi.advanceTimersByTimeAsync(0)))
    await emit(change("stt", "queued"), change("describe", "running"))
    expect(searchCalls()).toBe(0)
  })

  it("T-ui-aistate-14 [Time] many events inside the window collapse into one refresh", async () => {
    const { emit, searchCalls } = setup()
    await act(async () => void (await vi.advanceTimersByTimeAsync(0)))
    await emit(...Array.from({ length: 20 }, (_, i) => change("stt", "done", i + 1)))
    expect(searchCalls()).toBe(1)
  })
})
