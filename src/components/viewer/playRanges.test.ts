// Categories: Business rule (T-ui-ranges-01 shared vectors, -02..-04) / Boundary (-02..-04) / Validation (-05 rounding)
import { describe, expect, it } from "vitest"
import {
  formatRangeTime,
  mergeRanges,
  minRangeMs,
  nextRangeAction,
  nextRangeEnd,
  prevRangeStart,
  rangeIndexAt,
  roundRanges,
  totalRangesMs,
  videoLookaheadMs,
  type RangeAction,
} from "./playRanges"

const R = [
  { startMs: 1000, endMs: 2000 },
  { startMs: 5000, endMs: 6000 },
]
const NONE: RangeAction = { kind: "none" }

describe("nextRangeAction — shared vector table (design §4.3, identical rows to Rust t_player_ranges_01)", () => {
  const rows: [number, number, RangeAction][] = [
    [0, 50, { kind: "seek", toMs: 1000 }],
    [999, 50, { kind: "seek", toMs: 1000 }],
    [1000, 50, NONE],
    [1949, 50, NONE],
    [1950, 50, { kind: "seek", toMs: 5000 }],
    [3000, 50, { kind: "seek", toMs: 5000 }],
    [5949, 50, NONE],
    [5950, 50, { kind: "end", atMs: 6000 }],
    [9000, 50, { kind: "end", atMs: 6000 }],
    [1999, 0, NONE],
    [2000, 0, { kind: "seek", toMs: 5000 }],
  ]
  it.each(rows)("T-ui-ranges-01 [Business rule] t=%i lookahead=%i → %j", (t, lookahead, expected) => {
    // Arrange + Act + Assert
    expect(nextRangeAction(t, R, lookahead)).toEqual(expected)
  })

  it("T-ui-ranges-01 [Boundary] empty ranges always yield none, whatever the position", () => {
    // Arrange + Act + Assert
    for (const t of [0, 1, 50_000, -5]) expect(nextRangeAction(t, [], 50)).toEqual(NONE)
  })
})

describe("nextRangeAction — start tolerance (extra rows; NOT in the shared Rust vector table)", () => {
  const rows: [number, number, RangeAction][] = [
    [959, 40, { kind: "seek", toMs: 1000 }],
    [960, 40, NONE],
    [961, 40, NONE],
    [999.9999, 40, NONE],
    [1000, 40, NONE],
    [2000, 40, { kind: "seek", toMs: 5000 }], // 끝 이후엔 허용치가 영향 없다
    [4960, 40, NONE], // 구간 2 시작 직전 한 프레임 안
    [4959, 40, { kind: "seek", toMs: 5000 }],
  ]
  it.each(rows)("T-ui-ranges-06 [Boundary] t=%d tolerance=%i → %j", (t, tol, expected) => {
    // Arrange + Act + Assert
    expect(nextRangeAction(t, R, 0, tol)).toEqual(expected)
  })
})

describe("range helpers", () => {
  it("T-ui-ranges-02 [Boundary] minRangeMs: one frame at a valid fps, 100ms when fps is missing / zero / NaN; videoLookaheadMs is half a frame (17 fallback)", () => {
    // Arrange + Act + Assert
    expect(minRangeMs(30)).toBe(33)
    expect(minRangeMs(25)).toBe(40)
    expect(minRangeMs(2000)).toBe(1) // 1000/2000 = 0.5 → round 1 이상 보장
    for (const fps of [null, 0, -1, Number.NaN, Number.POSITIVE_INFINITY]) expect(minRangeMs(fps)).toBe(100)
    expect(videoLookaheadMs(25)).toBe(20)
    expect(videoLookaheadMs(null)).toBe(17)
    expect(videoLookaheadMs(0)).toBe(17)
  })

  it("T-ui-ranges-03 [Business rule] mergeRanges sorts, merges overlapping and touching ranges, keeps gaps; roundRanges makes integers; total / index lookups", () => {
    // Arrange
    const input = [
      { startMs: 5000, endMs: 6000 },
      { startMs: 0, endMs: 1000 },
      { startMs: 1000, endMs: 1500 },
      { startMs: 5500, endMs: 5600 },
    ]
    // Act
    const merged = mergeRanges(input)
    // Assert
    expect(merged).toEqual([
      { startMs: 0, endMs: 1500 },
      { startMs: 5000, endMs: 6000 },
    ])
    expect(input[0]).toEqual({ startMs: 5000, endMs: 6000 }) // 입력 불변
    expect(roundRanges([{ startMs: 100.4, endMs: 200.5 }])).toEqual([{ startMs: 100, endMs: 201 }])
    expect(totalRangesMs(merged)).toBe(2500)
    expect(rangeIndexAt(merged, 1500)).toBe(0)
    expect(rangeIndexAt(merged, 1501)).toBe(-1)
    expect(rangeIndexAt(merged, 5000)).toBe(1)
  })

  it("T-ui-ranges-04 [Boundary] prev start / next end skip the range the playhead is on (±250ms tolerance) and return null at the ends", () => {
    // Arrange: 구간 [1000,2000] [5000,6000]
    // Act + Assert: 이전 구간 시작
    expect(prevRangeStart(R, 5100)).toBe(1000) // 방금 이동한 5000 근처(허용 오차) → 그 앞 구간
    expect(prevRangeStart(R, 5300)).toBe(5000)
    expect(prevRangeStart(R, 1100)).toBeNull()
    // Act + Assert: 다음 구간 끝
    expect(nextRangeEnd(R, 1900)).toBe(6000) // 끝 근처(허용 오차)면 다음 구간
    expect(nextRangeEnd(R, 1500)).toBe(2000)
    expect(nextRangeEnd(R, 5900)).toBeNull()
    expect(prevRangeStart([], 1000)).toBeNull()
    expect(nextRangeEnd([], 1000)).toBeNull()
  })

  it("T-ui-ranges-04b [Boundary] formatRangeTime: tenths, minutes, hours, negatives clamp to zero", () => {
    // Arrange + Act + Assert
    expect(formatRangeTime(0)).toBe("0:00.0")
    expect(formatRangeTime(65_450)).toBe("1:05.4")
    expect(formatRangeTime(3_723_900)).toBe("1:02:03.9")
    expect(formatRangeTime(-50)).toBe("0:00.0")
  })
})
