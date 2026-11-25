// Categories: Happy (zones) / Boundary (0%, 60/70 edges, 99/100, invalid)
import { describe, expect, it } from "vitest"
import { BAND_CELLS, cellZone, markerCell, similarityZone } from "./bandZones"

describe("similarity band helpers", () => {
  it("T-ui-simband-01 [Boundary] the marker sits on the first cell at 0% and the last cell at 99% / 100%", () => {
    expect(markerCell(0)).toBe(0)
    expect(markerCell(99)).toBe(BAND_CELLS - 1)
    expect(markerCell(100)).toBe(BAND_CELLS - 1)
    expect(markerCell(250)).toBe(BAND_CELLS - 1)
  })

  it("T-ui-simband-02 [Happy] one cell per slider step: 75% -> cell 15, 5% -> cell 1", () => {
    expect(markerCell(75)).toBe(15)
    expect(markerCell(5)).toBe(1)
    expect(markerCell(60)).toBe(12)
  })

  it("T-ui-simband-03 [Validation] negative, NaN and Infinity input falls back safely", () => {
    expect(markerCell(-10)).toBe(0)
    expect(markerCell(Number.NaN)).toBe(0)
    expect(markerCell(Number.POSITIVE_INFINITY)).toBe(0)
  })

  it("T-ui-simband-04 [Business rule] 0..60% is weak, 61..69% is mid, 70%+ is recommended; cells use the same rule", () => {
    expect(similarityZone(0)).toBe("weak")
    expect(similarityZone(60)).toBe("weak")
    expect(similarityZone(65)).toBe("mid")
    expect(similarityZone(70)).toBe("recommended")
    expect(similarityZone(99)).toBe("recommended")
    expect([cellZone(0), cellZone(12), cellZone(13), cellZone(14), cellZone(19)]).toEqual(["weak", "weak", "mid", "recommended", "recommended"])
  })
})
