// Categories: Happy (restore, save on pause/unmount) / Business rule (gate ON/OFF, explicit seek wins, D5 ranges) / Boundary (no record, no save before metadata)
import { act, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { UiApi } from "@/lib/api"
import { mountHtmlPlayer, restoreFrames, type HtmlPlayerOpts } from "@/test/ranges"

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  restoreFrames()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RANGES = [
  { startMs: 10_000, endMs: 20_000 },
  { startMs: 40_000, endMs: 50_000 },
]

type Record = { positionMs: number; durationMs: number | null } | null

/** jsdom 은 readyState 가 항상 0 — 메타데이터가 이미 로드된 요소처럼 1 로 둔다 (실제 브라우저에서는 loadedmetadata 이전/이후 모두 이 경로) */
function stubReadyState(value = 1) {
  vi.spyOn(HTMLMediaElement.prototype, "readyState", "get").mockReturnValue(value)
}

async function mount(opts: HtmlPlayerOpts & { record?: Record }) {
  const { record = { positionMs: 60_000, durationMs: 100_000 }, ...rest } = opts
  const spies: { get?: ReturnType<typeof vi.spyOn>; save?: ReturnType<typeof vi.spyOn> } = {}
  const t = await mountHtmlPlayer({
    ...rest,
    setup: (api: UiApi) => {
      spies.get = vi.spyOn(api, "resumeGet").mockResolvedValue(record)
      spies.save = vi.spyOn(api, "resumeSave").mockResolvedValue("saved")
      rest.setup?.(api)
    },
  })
  return { ...t, get: spies.get!, save: spies.save! }
}

describe("<video>/<audio> resume", () => {
  it("T-ui-resume-40 [Happy] with a stored position the element seeks there once metadata is loaded, and the notice shows", async () => {
    // Arrange
    stubReadyState()
    // Act
    const t = await mount({ record: { positionMs: 60_000, durationMs: 100_000 } })
    // Assert
    await waitFor(() => expect(t.el.currentTime).toBe(60))
    expect(await screen.findByRole("status")).toHaveTextContent("1:00 부터 이어서 재생")
  })

  it("T-ui-resume-41 [Happy] the same restore applies to <audio>", async () => {
    // Arrange
    stubReadyState()
    // Act
    const t = await mount({ record: { positionMs: 42_000, durationMs: 100_000 }, detail: { mediaType: "audio", fileName: "clip.wav" } })
    // Assert
    expect(t.el.tagName).toBe("AUDIO")
    await waitFor(() => expect(t.el.currentTime).toBe(42))
  })

  it("T-ui-resume-42 [Boundary] no record → the element stays at 0 and no notice shows", async () => {
    // Arrange
    stubReadyState()
    // Act
    const t = await mount({ record: null })
    // Assert
    await waitFor(() => expect(t.get).toHaveBeenCalledWith(1))
    expect(t.el.currentTime).toBe(0)
    expect(screen.queryByText(/이어서 재생/)).toBeNull()
  })

  it("T-ui-resume-43 [Business rule] resumeEnabled OFF → no lookup and no restore", async () => {
    // Arrange
    stubReadyState()
    // Act
    const t = await mount({ settings: { resumeEnabled: false } })
    // Assert
    expect(t.get).not.toHaveBeenCalled()
    expect(t.el.currentTime).toBe(0)
  })

  it("T-ui-resume-44 [Business rule] an explicit seek request wins: no lookup, the element goes to the requested time (D6)", async () => {
    // Arrange
    stubReadyState()
    // Act
    const t = await mount({ seekMs: 5_000 })
    // Assert
    await waitFor(() => expect(t.el.currentTime).toBe(5))
    expect(t.get).not.toHaveBeenCalled()
  })

  it("T-ui-resume-45 [Business rule] ranges-only: a position between ranges restores to the next range start (D5); inside keeps it", async () => {
    // Arrange
    stubReadyState()
    // Act: 30s → 40s
    const outside = await mount({ record: { positionMs: 30_000, durationMs: 100_000 }, ranges: RANGES, settings: { playRangesOnly: true } })
    // Assert
    await waitFor(() => expect(outside.el.currentTime).toBe(40))
    outside.view.unmount()
    // Act: 45s 는 그대로
    const inside = await mount({ record: { positionMs: 45_000, durationMs: 100_000 }, ranges: RANGES, settings: { playRangesOnly: true } })
    await waitFor(() => expect(inside.el.currentTime).toBe(45))
  })

  it("T-ui-resume-46 [Happy] pausing saves the element position (and the duration); gate ON path", async () => {
    // Arrange
    stubReadyState()
    const t = await mount({ record: null })
    await waitFor(() => expect(t.get).toHaveBeenCalled())
    await act(async () => void (await Promise.resolve()))
    // Act: 재생 → 위치 이동 → 일시정지
    act(() => void t.el.play())
    t.el.currentTime = 33
    act(() => t.el.pause())
    // Assert
    await waitFor(() => expect(t.save).toHaveBeenCalledWith(1, 33_000, 100_000))
  })

  it("T-ui-resume-47 [Business rule] resumeEnabled OFF → pause and unmount save nothing (gate OFF)", async () => {
    // Arrange
    stubReadyState()
    const t = await mount({ settings: { resumeEnabled: false } })
    await act(async () => void (await Promise.resolve()))
    // Act
    act(() => void t.el.play())
    t.el.currentTime = 33
    act(() => t.el.pause())
    t.view.unmount()
    // Assert
    expect(t.save).not.toHaveBeenCalled()
  })

  it("T-ui-resume-48 [Boundary] metadata never loaded → unmount does not save a bogus 0 position over the stored record", async () => {
    // Arrange: readyState 0 + loadedmetadata 없음
    const t = await mount({ metadata: false })
    await waitFor(() => expect(t.get).toHaveBeenCalled())
    // Act
    t.view.unmount()
    // Assert
    expect(t.save).not.toHaveBeenCalled()
  })

  it("T-ui-resume-49 [Happy] closing after a restore saves the restored position", async () => {
    // Arrange
    stubReadyState()
    const t = await mount({ record: { positionMs: 60_000, durationMs: 100_000 } })
    await waitFor(() => expect(t.el.currentTime).toBe(60))
    // Act
    t.view.unmount()
    // Assert
    expect(t.save).toHaveBeenCalledWith(1, 60_000, 100_000)
  })
})
