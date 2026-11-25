// Categories: Happy (T-ui-rating-10, 12) / Boundary (T-ui-rating-11, 13)
import { describe, expect, it } from "vitest"
import { DEFAULT_FILTERS, activeFilterCount, effectiveFilters } from "./search"

describe("rating filter composition", () => {
  it("T-ui-rating-10 [Happy] a rating sidebar view maps to ratingMin = ratingMax = value", () => {
    // Arrange + Act
    const f = effectiveFilters({ kind: "rating", value: 4 }, DEFAULT_FILTERS)
    // Assert
    expect(f).toMatchObject({ ratingMin: 4, ratingMax: 4 })
  })

  it("T-ui-rating-11 [Boundary] value 0 (unrated) maps to 0/0, not null", () => {
    const f = effectiveFilters({ kind: "rating", value: 0 }, DEFAULT_FILTERS)
    expect(f.ratingMin).toBe(0)
    expect(f.ratingMax).toBe(0)
  })

  it("T-ui-rating-12 [Happy] the rating range counts as exactly one active filter", () => {
    expect(activeFilterCount(DEFAULT_FILTERS)).toBe(0)
    expect(activeFilterCount({ ...DEFAULT_FILTERS, ratingMin: 3 })).toBe(1)
    expect(activeFilterCount({ ...DEFAULT_FILTERS, ratingMin: 3, ratingMax: 5 })).toBe(1)
    expect(activeFilterCount({ ...DEFAULT_FILTERS, ratingMax: 0 })).toBe(1)
  })

  it("T-ui-rating-13 [Boundary] other views leave the user's rating range untouched", () => {
    const user = { ...DEFAULT_FILTERS, ratingMin: 2, ratingMax: 3 }
    expect(effectiveFilters({ kind: "favorites" }, user)).toMatchObject({ ratingMin: 2, ratingMax: 3 })
  })
})
