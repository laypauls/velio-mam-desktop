// Categories: Happy (T-ui-fmt-01) / Boundary (T-ui-fmt-02)
import { describe, expect, it } from "vitest"
import { formatDate, formatDateCompact } from "./format"

describe("formatDateCompact", () => {
  it("T-ui-fmt-01 [Happy] renders YYYY.MM.DD with zero padding, shorter than formatDate", () => {
    // Arrange
    const ms = new Date(2026, 10, 7, 12).getTime() // 로컬 시간대 기준
    // Act + Assert
    expect(formatDateCompact(ms)).toBe("2026.11.07")
    expect(formatDateCompact(ms).length).toBeLessThan(formatDate(ms).length)
  })

  it("T-ui-fmt-02 [Boundary] first and last day of the year keep two-digit month/day; formatDate itself is unchanged", () => {
    // Arrange + Act + Assert
    expect(formatDateCompact(new Date(2026, 0, 1, 12).getTime())).toBe("2026.01.01")
    expect(formatDateCompact(new Date(2026, 11, 31, 12).getTime())).toBe("2026.12.31")
    expect(formatDate(new Date(2026, 10, 7, 12).getTime())).toBe("2026. 11. 07.")
  })
})
