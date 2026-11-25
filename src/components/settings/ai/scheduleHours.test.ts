// Categories: Happy (window) / Boundary (wrap, all day, edges, out of range)
import { describe, expect, it } from "vitest"
import { activeHours, scheduleSegments } from "./scheduleHours"

const on = (start: number, end: number) => activeHours(start, end).flatMap((a, h) => (a ? [h] : []))

describe("scheduleSegments / activeHours", () => {
  it("T-ui-sched-01 [Happy] a same-day window is one segment, start inclusive and end exclusive", () => {
    expect(scheduleSegments(9, 17)).toEqual({ kind: "window", segments: [{ from: 9, to: 17 }] })
    expect(on(9, 17)).toEqual([9, 10, 11, 12, 13, 14, 15, 16])
  })

  it("T-ui-sched-02 [Boundary] 22 -> 6 wraps past midnight into two segments covering 22..23 and 0..5", () => {
    expect(scheduleSegments(22, 6)).toEqual({
      kind: "wrap",
      segments: [
        { from: 22, to: 24 },
        { from: 0, to: 6 },
      ],
    })
    expect(on(22, 6)).toEqual([0, 1, 2, 3, 4, 5, 22, 23])
  })

  it("T-ui-sched-03 [Boundary] start == end means the whole day, including the default 0 -> 0", () => {
    for (const h of [0, 6, 23]) {
      expect(scheduleSegments(h, h).kind).toBe("allDay")
      expect(on(h, h)).toHaveLength(24)
    }
  })

  it("T-ui-sched-04 [Boundary] edges: 0 -> 24 is all day, 0 -> 1 is one hour, 23 -> 0 stops at midnight without an empty tail", () => {
    expect(scheduleSegments(0, 24).kind).toBe("allDay")
    expect(on(0, 1)).toEqual([0])
    expect(scheduleSegments(23, 0)).toEqual({ kind: "window", segments: [{ from: 23, to: 24 }] })
    expect(on(23, 0)).toEqual([23])
  })

  it("T-ui-sched-05 [Validation] out-of-range and fractional input is clamped to 0..24 and never throws", () => {
    expect(on(-5, 3)).toEqual([0, 1, 2])
    expect(scheduleSegments(30, 30).kind).toBe("allDay")
    expect(on(5.9, 8)).toEqual([5, 6, 7])
  })
})
