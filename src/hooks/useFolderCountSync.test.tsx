// Categories: Happy (T-ui-foldersync-01, 02) / Business rule (03) / Time (04) / Concurrency-cleanup (05) / Boundary (06)
import { act, screen, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { createMockApi } from "@/lib/api/mock"
import type { UiApi } from "@/lib/api"
import type { JobSummary, ManagedFolder } from "@/lib/ipc-contract"
import { renderApp, stubLayout } from "@/test/layout"
import { FOLDER_COUNT_SYNC_MS, folderCountsMayHaveChanged } from "./useFolderCountSync"

type Handler = (s: JobSummary) => void

/** StatusBar 가 모든 종류의 카운트를 읽는다 — mock 의 실제 형태를 한 번 받아 재사용 */
let COUNTS: JobSummary["counts"]

interface Harness {
  emit: (s: JobSummary) => Promise<void>
  advance: (ms: number) => Promise<void>
  listFolders: ReturnType<typeof vi.fn>
  setCount: (n: number) => void
  unmount: () => void
  row: () => HTMLElement
}

async function setup(): Promise<Harness> {
  stubLayout()
  const base = createMockApi({ count: 10 })
  const [folder] = await base.listFolders()
  COUNTS = (await base.jobSummary()).counts
  let count = 0
  const handlers = new Set<Handler>()
  const listFolders = vi.fn(async (): Promise<ManagedFolder[]> => [{ ...folder, assetCount: count }])
  const api: UiApi = {
    ...base,
    listFolders,
    onJobsProgress: async (h) => {
      handlers.add(h)
      return () => void handlers.delete(h)
    },
  }
  const view = renderApp(api)
  const row = () => within(screen.getByRole("navigation", { name: "라이브러리 탐색" })).getByRole("button", { name: folder.path }).closest("li") as HTMLElement
  await within(await screen.findByRole("navigation", { name: "라이브러리 탐색" })).findByRole("button", { name: folder.path })
  vi.useFakeTimers()
  const advance = (ms: number) =>
    act(async () => {
      await vi.advanceTimersByTimeAsync(ms)
      await vi.advanceTimersByTimeAsync(10) // 재조회 응답·렌더 반영
    })
  const emit = (s: JobSummary) => act(async () => handlers.forEach((h) => h(s)))
  await advance(0)
  return { emit, advance, listFolders, setCount: (n) => (count = n), unmount: view.unmount, row }
}

const summary = (totalAssets: number, scanning: boolean, processed = 0): JobSummary => ({
  paused: false,
  pausedKinds: [],
  counts: COUNTS,
  scan: scanning ? { folderId: 1, folderPath: "D:\\Photos", processed, total: null } : null,
  totalAssets,
})

beforeEach(() => vi.useRealTimers())
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("sidebar folder count follows scan/watch changes", () => {
  it("T-ui-foldersync-01 [Happy] count 0 -> totalAssets grows -> count updates after the throttle", async () => {
    // Arrange
    const h = await setup()
    await h.emit(summary(0, true)) // baseline
    h.setCount(767)
    const calls = h.listFolders.mock.calls.length
    // Act
    await h.emit(summary(767, true, 767))
    expect(within(h.row()).queryByText("767")).not.toBeInTheDocument()
    await h.advance(FOLDER_COUNT_SYNC_MS)
    // Assert
    expect(h.listFolders.mock.calls.length).toBe(calls + 1)
    expect(within(h.row()).getByText("767")).toBeInTheDocument()
  })

  it("T-ui-foldersync-02 [Happy] scan end (non-null -> null) refreshes with the final value", async () => {
    // Arrange
    const h = await setup()
    await h.emit(summary(5, true, 5)) // baseline
    h.setCount(5)
    const calls = h.listFolders.mock.calls.length
    // Act — totalAssets unchanged, only the scan ends
    await h.emit(summary(5, false))
    await h.advance(FOLDER_COUNT_SYNC_MS)
    // Assert
    expect(h.listFolders.mock.calls.length).toBe(calls + 1)
    expect(within(h.row()).getByText("5")).toBeInTheDocument()
  })

  it("T-ui-foldersync-03 [Business rule] progress-only events (same totalAssets, scan still running) never refetch", async () => {
    // Arrange
    const h = await setup()
    await h.emit(summary(3, true, 1)) // baseline
    const calls = h.listFolders.mock.calls.length
    // Act
    for (let i = 2; i < 12; i++) await h.emit(summary(3, true, i))
    await h.advance(FOLDER_COUNT_SYNC_MS * 3)
    // Assert
    expect(h.listFolders.mock.calls.length).toBe(calls)
  })

  it("T-ui-foldersync-04 [Time] a burst inside the window yields one refetch and applies the last change", async () => {
    // Arrange
    const h = await setup()
    await h.emit(summary(0, true)) // baseline
    const calls = h.listFolders.mock.calls.length
    // Act
    for (let n = 1; n <= 20; n++) {
      h.setCount(n)
      await h.emit(summary(n, true, n))
      await h.advance(50)
    }
    await h.advance(FOLDER_COUNT_SYNC_MS)
    // Assert
    expect(h.listFolders.mock.calls.length).toBe(calls + 1)
    expect(within(h.row()).getByText("20")).toBeInTheDocument()
  })

  it("T-ui-foldersync-05 [Concurrency] unmount clears the pending timer and the listener", async () => {
    // Arrange
    const h = await setup()
    await h.emit(summary(0, true)) // baseline
    await h.emit(summary(9, true)) // schedules a refetch
    const calls = h.listFolders.mock.calls.length
    // Act
    h.unmount()
    await h.advance(FOLDER_COUNT_SYNC_MS * 2)
    // Assert
    expect(h.listFolders.mock.calls.length).toBe(calls)
  })

  it("T-ui-foldersync-06 [Boundary] only totalAssets change or scan end counts as a change", () => {
    // Arrange / Act / Assert
    expect(folderCountsMayHaveChanged(summary(1, true), summary(1, true, 9))).toBe(false)
    expect(folderCountsMayHaveChanged(summary(1, false), summary(1, false))).toBe(false)
    expect(folderCountsMayHaveChanged(summary(1, false), summary(1, true))).toBe(false)
    expect(folderCountsMayHaveChanged(summary(1, true), summary(1, false))).toBe(true)
    expect(folderCountsMayHaveChanged(summary(1, false), summary(0, false))).toBe(true)
  })
})
