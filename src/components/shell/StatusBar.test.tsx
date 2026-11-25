// Categories: Happy (T-ui-status-01) / Boundary (T-ui-status-02, 03) / Business rule (T-ui-status-04, 05, 06) / External (T-ui-status-07)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { JobCounts, JobKind, JobSummary } from "@/lib/ipc-contract"
import { StatusBar } from "./StatusBar"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }))

afterEach(() => vi.clearAllMocks())

const ZERO: JobCounts = { pending: 0, running: 0, failed: 0 }

function summary(counts: Partial<Record<JobKind, Partial<JobCounts>>> = {}, pausedKinds: JobKind[] = []): JobSummary {
  const kinds: JobKind[] = ["scan", "meta", "thumb", "hash", "index", "ai", "scene", "plugin"]
  return {
    paused: false,
    pausedKinds,
    counts: Object.fromEntries(kinds.map((k) => [k, { ...ZERO, ...counts[k] }])) as Record<JobKind, JobCounts>,
    scan: null,
    totalAssets: 0,
  }
}

function setup(s: JobSummary) {
  const api = createMockApi({ count: 10 })
  vi.spyOn(api, "jobSummary").mockResolvedValue(s)
  vi.spyOn(api, "onJobsProgress").mockResolvedValue(() => {})
  const pauseJobs = vi.spyOn(api, "pauseJobs").mockResolvedValue(s)
  const resumeJobs = vi.spyOn(api, "resumeJobs").mockResolvedValue(s)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  render(
    <QueryClientProvider client={queryClient}>
      <ApiProvider api={api}>
        <StatusBar />
      </ApiProvider>
    </QueryClientProvider>
  )
  return { pauseJobs, resumeJobs, user: userEvent.setup() }
}

const openDetail = async (user: ReturnType<typeof userEvent.setup>, chip: string | RegExp) => {
  await user.click(await screen.findByRole("button", { name: chip }))
  return screen.findByRole("table")
}

describe("StatusBar background queue", () => {
  it("T-ui-status-01 [Happy] shows pending AI work as a chip", async () => {
    setup(summary({ ai: { pending: 340 } }))
    expect(await screen.findByRole("button", { name: /AI 분석 340/ })).toBeTruthy()
  })

  it("T-ui-status-02 [Boundary] renders no chip trigger when every count is zero", async () => {
    setup(summary())
    await screen.findByRole("button", { name: "작업 일시정지" })
    expect(screen.queryByText(/AI 분석|썸네일|메타데이터|중복 검사/)).toBeNull()
  })

  it("T-ui-status-03 [Boundary] formats large counts with thousands separators", async () => {
    setup(summary({ thumb: { pending: 1_000_000, running: 234 } }))
    expect(await screen.findByRole("button", { name: /썸네일 1,000,234/ })).toBeTruthy()
  })

  it("T-ui-status-04 [Business rule] scan and index counts never produce chips or table rows", async () => {
    const { user } = setup(summary({ scan: { pending: 9 }, index: { pending: 7 }, meta: { pending: 1 } }))
    const table = await openDetail(user, /메타데이터 1/)
    expect(within(table).getAllByRole("row")).toHaveLength(7) // header + meta/thumb/hash/ai/scene/plugin
    expect(screen.queryByText(/인덱스|index/i)).toBeNull()
  })

  it("T-ui-status-05 [Business rule] table lists all four kinds even when zero, and the trigger opens it", async () => {
    const { user } = setup(summary({ ai: { pending: 2, running: 1, failed: 7 } }))
    const table = await openDetail(user, /AI 분석 3/)
    for (const name of ["메타데이터", "썸네일", "중복 검사", "AI 분석"]) {
      expect(within(table).getByRole("rowheader", { name })).toBeTruthy()
    }
    expect(screen.getByRole("button", { name: "썸네일 일시정지" })).toBeTruthy()
  })

  it("T-ui-scene-09a [Happy] the scene kind shows as \"장면 검출\" with its count, has a table row, and pauses / resumes through pauseJobs/resumeJobs(\"scene\")", async () => {
    // Arrange
    const { user, pauseJobs } = setup(summary({ scene: { pending: 300, running: 1 } }))
    // Act
    const table = await openDetail(user, /장면 검출 301/)
    await user.click(screen.getByRole("button", { name: "장면 검출 일시정지" }))
    // Assert
    expect(within(table).getByRole("rowheader", { name: "장면 검출" })).toBeTruthy()
    expect(pauseJobs).toHaveBeenCalledWith("scene")
  })

  it("T-ui-scene-09b [Business rule] a paused scene kind with zero waiting stays visible and offers resume", async () => {
    const { user, resumeJobs } = setup(summary({}, ["scene"]))
    await openDetail(user, /장면 검출 0/)
    await user.click(screen.getByRole("button", { name: "장면 검출 재개" }))
    expect(resumeJobs).toHaveBeenCalledWith("scene")
  })

  it("T-ui-plugin-15 [Happy] the plugin kind shows as \"플러그인\" with its count, has a table row, and pauses through pauseJobs(\"plugin\")", async () => {
    // Arrange
    const { user, pauseJobs } = setup(summary({ plugin: { pending: 12, running: 1, failed: 2 } }))
    // Act
    const table = await openDetail(user, /플러그인 13/)
    await user.click(screen.getByRole("button", { name: "플러그인 일시정지" }))
    // Assert
    const row = within(table).getByRole("rowheader", { name: "플러그인" }).closest("tr")!
    expect(within(row).getAllByRole("cell").map((c) => c.textContent).slice(0, 3)).toEqual(["12", "1", "2"])
    expect(pauseJobs).toHaveBeenCalledWith("plugin")
  })

  it("T-ui-status-06 [Business rule] a paused kind offers resume and calls resumeJobs(kind)", async () => {
    const { user, resumeJobs, pauseJobs } = setup(summary({ hash: { pending: 5 } }, ["hash"]))
    await openDetail(user, /중복 검사 5/)
    await user.click(screen.getByRole("button", { name: "중복 검사 재개" }))
    expect(resumeJobs).toHaveBeenCalledWith("hash")
    expect(pauseJobs).not.toHaveBeenCalled()
  })

  it("T-ui-status-08 [Business rule] a paused kind with zero waiting stays visible and can be resumed", async () => {
    const { user, resumeJobs } = setup(summary({}, ["ai"]))
    await openDetail(user, /AI 분석 0/)
    await user.click(screen.getByRole("button", { name: "AI 분석 재개" }))
    expect(resumeJobs).toHaveBeenCalledWith("ai")
  })

  it("T-ui-status-07[External] pauseJobs(kind) rejection surfaces a toast", async () => {
    const { user, pauseJobs } = setup(summary({ ai: { pending: 4 } }))
    pauseJobs.mockRejectedValueOnce({ code: "internal", message: "boom" })
    await openDetail(user, /AI 분석 4/)
    await user.click(screen.getByRole("button", { name: "AI 분석 일시정지" }))
    expect(pauseJobs).toHaveBeenCalledWith("ai")
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
  })
})
