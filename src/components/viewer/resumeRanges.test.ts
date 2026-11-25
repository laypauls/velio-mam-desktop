// Categories: Business rule (D5) / Boundary (range edges, empty ranges)
import { describe, expect, it } from "vitest"
import { fitResumeToRanges, resumeRestartTarget } from "./resumeRanges"

const RANGES = [
  { startMs: 10_000, endMs: 20_000 },
  { startMs: 40_000, endMs: 50_000 },
]

describe("fitResumeToRanges (D5)", () => {
  it("T-ui-resume-11 [Business rule] inside a range keeps the position", () => {
    expect(fitResumeToRanges(15_000, RANGES)).toBe(15_000)
    expect(fitResumeToRanges(45_000, RANGES)).toBe(45_000)
  })

  it("T-ui-resume-12 [Business rule] outside → start of the next range; between ranges and before the first", () => {
    expect(fitResumeToRanges(30_000, RANGES)).toBe(40_000)
    expect(fitResumeToRanges(2_000, RANGES)).toBe(10_000)
  })

  it("T-ui-resume-13 [Business rule] no range after the position → first range start", () => {
    expect(fitResumeToRanges(80_000, RANGES)).toBe(10_000)
  })

  it("T-ui-resume-14 [Boundary] start is inside, end is outside (half-open); null / [] means whole-file playback", () => {
    expect(fitResumeToRanges(10_000, RANGES)).toBe(10_000)
    expect(fitResumeToRanges(20_000, RANGES)).toBe(40_000)
    expect(fitResumeToRanges(30_000, null)).toBe(30_000)
    expect(fitResumeToRanges(30_000, undefined)).toBe(30_000)
    expect(fitResumeToRanges(30_000, [])).toBe(30_000)
  })

  it("T-ui-resume-15 [Business rule] restart target is the first range start, or 0 without ranges", () => {
    expect(resumeRestartTarget(RANGES)).toBe(10_000)
    expect(resumeRestartTarget(null)).toBe(0)
    expect(resumeRestartTarget([])).toBe(0)
  })
})
