// Categories: Happy (restore, save on pause) / Business rule (gate ON/OFF, explicit seek wins, D5 ranges) / Boundary (no record, no save before open) / External (lookup failure)
import { fireEvent, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { UiApi } from "@/lib/api"
import { useViewStore } from "@/stores"
import { mountMpv, mpvQueueItem, type MpvOpts } from "@/test/mpv"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RECORD = { positionMs: 754_000, durationMs: 1_200_000 }
const RANGES = [
  { startMs: 10_000, endMs: 20_000 },
  { startMs: 40_000, endMs: 50_000 },
]

/** resumeGet / resumeSave 에 스파이를 건다 (렌더 전) */
function withResume(record: typeof RECORD | null | Error = RECORD) {
  const spies: { get?: ReturnType<typeof vi.spyOn>; save?: ReturnType<typeof vi.spyOn> } = {}
  const setup = (api: UiApi) => {
    spies.get = vi.spyOn(api, "resumeGet").mockImplementation(() => (record instanceof Error ? Promise.reject(record) : Promise.resolve(record)))
    spies.save = vi.spyOn(api, "resumeSave").mockResolvedValue("saved")
  }
  return { setup, spies }
}

async function mount(opts: MpvOpts & { record?: typeof RECORD | null | Error; staleSeekMs?: number }) {
  const { record, staleSeekMs, ...rest } = opts
  const r = withResume(record === undefined ? RECORD : record)
  const t = await mountMpv({
    reply: { timeMs: 0 },
    ...rest,
    setup: (api) => {
      r.setup(api)
      if (staleSeekMs !== undefined) useViewStore.setState({ seekMs: staleSeekMs }) // 이전 미리보기에서 남은 시각 지정
    },
  })
  return { ...t, get: r.spies.get!, save: r.spies.save! }
}

describe("built-in player resume", () => {
  it("T-ui-resume-20 [Happy] a stored position is passed to player_open as startMs, and the 이어서 재생 notice shows", async () => {
    // Arrange + Act
    const t = await mount({ reply: { timeMs: 754_000 } })
    await t.ready()
    // Assert
    expect(t.get).toHaveBeenCalledWith(1)
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(t.open.mock.calls[0].slice(3)).toEqual([undefined, 754_000])
    expect(await screen.findByRole("status")).toHaveTextContent("12:34 부터 이어서 재생")
  })

  it("T-ui-resume-21 [Boundary] no stored record → player_open has no startMs (3 args) and no notice", async () => {
    // Arrange + Act
    const t = await mount({ record: null })
    await t.ready()
    // Assert
    expect(t.get).toHaveBeenCalledTimes(1)
    expect(t.open.mock.calls[0]).toHaveLength(3)
    expect(screen.queryByText(/이어서 재생/)).toBeNull()
  })

  it("T-ui-resume-22 [Business rule] resumeEnabled OFF → no lookup, no startMs, no notice", async () => {
    // Arrange + Act
    const t = await mount({ settings: { resumeEnabled: false } })
    await t.ready()
    // Assert
    expect(t.get).not.toHaveBeenCalled()
    expect(t.open.mock.calls[0]).toHaveLength(3)
    expect(screen.queryByText(/이어서 재생/)).toBeNull()
  })

  it("T-ui-resume-23 [Business rule] an explicit seek request (subtitle / search hit) wins over resume (D6)", async () => {
    // Arrange + Act
    const t = await mount({ seekMs: 5_000 })
    await t.ready()
    // Assert
    expect(t.get).not.toHaveBeenCalled()
    expect(t.open.mock.calls[0]).toHaveLength(3)
    await waitFor(() => expect(t.seek).toHaveBeenCalledWith(5_000))
  })

  it("T-ui-resume-24 [Business rule] ranges-only: a position outside the ranges starts at the next range (D5), inside keeps it", async () => {
    // Arrange + Act: 30s 는 구간 사이 → 40s
    const outside = await mount({ record: { positionMs: 30_000, durationMs: 600_000 }, ranges: RANGES, settings: { playRangesOnly: true }, prefetchRanges: true, reply: { timeMs: 40_000 } })
    await outside.ready()
    // Assert
    expect(outside.open.mock.calls[0][3]).toEqual(RANGES)
    expect(outside.open.mock.calls[0][4]).toBe(40_000)
    outside.view.unmount()
    // Arrange + Act: 45s 는 구간 안
    const inside = await mount({ record: { positionMs: 45_000, durationMs: 600_000 }, ranges: RANGES, settings: { playRangesOnly: true }, prefetchRanges: true, reply: { timeMs: 45_000 } })
    await inside.ready()
    expect(inside.open.mock.calls[0][4]).toBe(45_000)
  })

  it("T-ui-resume-25 [Happy] 처음부터 seeks to 0 and dismisses the notice", async () => {
    // Arrange
    const t = await mount({ reply: { timeMs: 754_000 } })
    await t.ready()
    // Act
    fireEvent.click(await screen.findByRole("button", { name: "처음부터" }))
    // Assert
    await waitFor(() => expect(t.seek).toHaveBeenCalledWith(0))
    expect(screen.queryByText(/이어서 재생/)).toBeNull()
  })

  it("T-ui-resume-26 [Business rule] no save while the open is still pending (no bogus 0 position), even when unmounted", async () => {
    // Arrange
    const t = await mount({ open: "pending", record: null })
    await waitFor(() => expect(t.open).toHaveBeenCalled())
    // Act
    t.view.unmount()
    t.finishOpen()
    // Assert
    expect(t.save).not.toHaveBeenCalled()
  })

  it("T-ui-resume-27 [Happy] playing → paused saves the worker position with the length", async () => {
    // Arrange
    const t = await mount({ record: null, reply: { state: "playing", timeMs: 0 } })
    await t.ready()
    t.emitState({ state: "playing", timeMs: 30_000 })
    // Act
    t.emitState({ state: "paused", timeMs: 31_000 })
    // Assert
    await waitFor(() => expect(t.save).toHaveBeenCalledWith(1, 31_000, 120_000))
  })

  it("T-ui-resume-28 [Happy] closing the viewer after opening saves the restored position (unmount)", async () => {
    // Arrange
    const t = await mount({ record: { positionMs: 90_000, durationMs: 120_000 }, reply: { timeMs: 90_000 } })
    await t.ready()
    await screen.findByLabelText("1:30 / 2:00") // 열기 응답의 위치가 상태에 반영된 뒤(저장 문턱)
    // Act
    t.view.unmount()
    // Assert
    expect(t.save).toHaveBeenCalledWith(1, 90_000, 120_000)
  })

  it("T-ui-resume-29 [Business rule] resumeEnabled OFF → nothing is saved on pause or close", async () => {
    // Arrange
    const t = await mount({ settings: { resumeEnabled: false }, reply: { state: "playing", timeMs: 0 } })
    await t.ready()
    t.emitState({ state: "playing", timeMs: 30_000 })
    // Act
    t.emitState({ state: "paused", timeMs: 31_000 })
    t.view.unmount()
    // Assert
    expect(t.save).not.toHaveBeenCalled()
  })

  it("T-ui-resume-30 [External] a failing lookup still opens from the start, and nothing is saved until the user plays (an existing record is not wiped)", async () => {
    // Arrange
    const t = await mount({ record: new Error("boom") })
    await t.ready()
    expect(t.open.mock.calls[0]).toHaveLength(3)
    // Act: 재생 전에 닫는다
    t.view.unmount()
    // Assert
    expect(t.save).not.toHaveBeenCalled()
  })

  it("T-ui-resume-31 [Business rule] continuous playback (D6): the queue's current item resumes even when a stale seekMs is left in the view store", async () => {
    // Arrange + Act
    const t = await mount({ queue: [mpvQueueItem(1), mpvQueueItem(2)], staleSeekMs: 9_000, reply: { timeMs: 754_000 } })
    await t.ready()
    // Assert
    expect(t.get).toHaveBeenCalledWith(1)
    expect(t.open.mock.calls[0][4]).toBe(754_000)
  })
})
