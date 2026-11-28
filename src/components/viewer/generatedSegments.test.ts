// Categories: Boundary (T-ui-gensub-01, 02) / Validation (T-ui-gensub-03)
import { describe, expect, it } from "vitest"
import { findActiveSegment, sortSegments } from "./generatedSegments"

const A = { startMs: 1000, endMs: 2000, text: "A" }
const B = { startMs: 3000, endMs: 4000, text: "B" }
const C = { startMs: 4000, endMs: 5000, text: "C" }

describe("findActiveSegment", () => {
  it("T-ui-gensub-01 [Boundary] start is inclusive and end is exclusive; a segment that ends where the next starts hands over exactly at the boundary", () => {
    // Arrange
    const segs = [A, B, C]
    // Act + Assert
    expect(findActiveSegment(segs, 1000)).toBe(A)
    expect(findActiveSegment(segs, 1999)).toBe(A)
    expect(findActiveSegment(segs, 2000)).toBeNull()
    expect(findActiveSegment(segs, 3999)).toBe(B)
    expect(findActiveSegment(segs, 4000)).toBe(C)
    expect(findActiveSegment(segs, 5000)).toBeNull()
  })

  it("T-ui-gensub-02 [Boundary] before the first segment, in a gap, after the last, with no segments → null", () => {
    // Arrange + Act + Assert
    expect(findActiveSegment([A, B], 0)).toBeNull()
    expect(findActiveSegment([A, B], 2500)).toBeNull()
    expect(findActiveSegment([A, B], 9_000_000)).toBeNull()
    expect(findActiveSegment([], 1000)).toBeNull()
  })

  it("T-ui-gensub-02 [Boundary] a single segment starting at 0 is found at time 0", () => {
    // Arrange
    const first = { startMs: 0, endMs: 500, text: "처음" }
    // Act + Assert
    expect(findActiveSegment([first], 0)).toBe(first)
    expect(findActiveSegment([first], 500)).toBeNull()
  })
})

describe("sortSegments", () => {
  it("T-ui-gensub-03 [Validation] unsorted input is sorted into a copy (input untouched) so lookup still works", () => {
    // Arrange
    const input = [C, A, B]
    // Act
    const sorted = sortSegments(input)
    // Assert
    expect(sorted).toEqual([A, B, C])
    expect(input).toEqual([C, A, B])
    expect(findActiveSegment(sorted, 3500)).toBe(B)
  })

  it("T-ui-gensub-03 [Business rule] already sorted input is returned as is (no copy)", () => {
    // Arrange
    const input = [A, B, C]
    // Act + Assert
    expect(sortSegments(input)).toBe(input)
  })
})
