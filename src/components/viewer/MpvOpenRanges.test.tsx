// Categories: Happy (T-ui-ranges-35) / Business rule (T-ui-ranges-36, 37) / Boundary (T-ui-ranges-38 rounding) / External (T-ui-ranges-39 open failure)
import { fireEvent, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { mountMpv } from "@/test/mpv"
import { toWireRanges } from "./useMpvSession"

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const RANGES = [
  { startMs: 10_000, endMs: 20_000 },
  { startMs: 40_000, endMs: 50_000 },
]

describe("built-in player open with ranges (player_open { ranges })", () => {
  it("T-ui-ranges-35 [Happy] ranges-only on and ranges already loaded → player_open carries them, and no redundant player_set_ranges follows", async () => {
    // Arrange + Act
    const t = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: true }, prefetchRanges: true })
    await t.ready()
    // Assert
    expect(t.open).toHaveBeenCalledTimes(1)
    expect(t.open.mock.calls[0][3]).toEqual(RANGES)
    expect(t.setPlayerRanges).not.toHaveBeenCalled()
  })

  it("T-ui-ranges-36 [Business rule] after an open that carried ranges, an edit and a toggle-off still reach the player via player_set_ranges", async () => {
    // Arrange
    const t = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: true }, prefetchRanges: true })
    await t.ready()
    expect(t.setPlayerRanges).not.toHaveBeenCalled()
    // Act 1: 끔
    fireEvent.click(await screen.findByRole("button", { name: "구간만" }))
    // Assert 1
    await waitFor(() => expect(t.setPlayerRanges).toHaveBeenLastCalledWith(1, null))
    // Act 2: 다시 켬
    fireEvent.click(screen.getByRole("button", { name: "구간만" }))
    // Assert 2
    await waitFor(() => expect(t.setPlayerRanges).toHaveBeenLastCalledWith(1, RANGES))
    expect(t.setPlayerRanges).toHaveBeenCalledTimes(2)
  })

  it("T-ui-ranges-37 [Business rule] ranges not loaded at open, toggle off, or no ranges (D7) → player_open has no ranges argument (current behaviour: send after load)", async () => {
    // Arrange + Act: 구간이 아직 안 왔다 (미리 받지 않음)
    // 이어보기 조회가 끝난 뒤에 열리므로 구간이 열기 전에 도착한다 — 이 시험의 전제(열기 뒤 도착)를 유지하려고 이어보기를 끈다
    const lazy = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: true, resumeEnabled: false } })
    await lazy.ready()
    // Assert 1: 열기에는 없고 로드 뒤 set_ranges 로 간다
    expect(lazy.open.mock.calls[0]).toHaveLength(3)
    await waitFor(() => expect(lazy.setPlayerRanges).toHaveBeenCalledWith(1, RANGES))
    lazy.view.unmount()
    // Arrange 2: 토글 꺼짐
    const off = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: false }, prefetchRanges: true })
    await off.ready()
    expect(off.open.mock.calls[0]).toHaveLength(3)
    off.view.unmount()
    // Arrange 3: 구간 없음
    const none = await mountMpv({ ranges: [], settings: { playRangesOnly: true }, prefetchRanges: true })
    await none.ready()
    expect(none.open.mock.calls[0]).toHaveLength(3)
    expect(none.setPlayerRanges).not.toHaveBeenCalled()
  })

  it("T-ui-ranges-38 [Boundary] wire ranges are rounded to integer ms; empty / null / undefined mean off", () => {
    // Arrange + Act + Assert: 서버는 정수만 주지만 와이어 값은 항상 반올림한다 (소수 ms 는 command 진입 전에 거부된다)
    expect(toWireRanges([{ startMs: 10_000.4, endMs: 20_000.6 }])).toEqual([{ startMs: 10_000, endMs: 20_001 }])
    expect(toWireRanges([])).toBeNull()
    expect(toWireRanges(null)).toBeNull()
    expect(toWireRanges(undefined)).toBeNull()
  })

  it("T-ui-ranges-39 [External] a failing open that carried ranges shows the failure notice and sends no player_set_ranges", async () => {
    // Arrange
    const t = await mountMpv({ ranges: RANGES, settings: { playRangesOnly: true }, prefetchRanges: true, open: "fail" })
    // Act + Assert
    await waitFor(() => expect(t.open).toHaveBeenCalled())
    expect(await screen.findByRole("alert")).toBeInTheDocument()
    expect(t.setPlayerRanges).not.toHaveBeenCalled()
  })
})
