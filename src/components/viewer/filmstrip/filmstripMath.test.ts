// Categories: Boundary (T-ui-filmstrip-math-01 aspect clamp, 02 height fit, 04 ends, 05 empty) / Happy (T-ui-filmstrip-math-03 window)
import { describe, expect, it } from "vitest"
import { STAGE_MIN_HEIGHT, GRIP_HEIGHT, TOOLBAR_HEIGHT, TILE_GAP, effectiveStripHeight, stripWindow, tileAspect, tileWidth } from "./filmstripMath"

describe("filmstripMath", () => {
  it("T-ui-filmstrip-math-01 [Boundary] tile aspect is clamped to 0.75..1.78; audio and unknown size are square", () => {
    // Arrange + Act + Assert
    expect(tileAspect({ mediaType: "image", width: 1000, height: 4000 })).toBe(0.75)
    expect(tileAspect({ mediaType: "video", width: 3840, height: 1080 })).toBe(1.78)
    expect(tileAspect({ mediaType: "image", width: 300, height: 200 })).toBe(1.5)
    expect(tileAspect({ mediaType: "audio", width: 300, height: 200 })).toBe(1)
    expect(tileAspect({ mediaType: "image", width: null, height: null })).toBe(1)
    expect(tileAspect(undefined)).toBe(1)
    expect(tileWidth(92, { mediaType: "image", width: 200, height: 100 })).toBe(Math.round(92 * 1.78))
  })

  it("T-ui-filmstrip-math-02 [Business rule] the stage keeps 240px first: the strip shrinks toward 72 and unknown container height keeps the stored value", () => {
    const need = TOOLBAR_HEIGHT + GRIP_HEIGHT + STAGE_MIN_HEIGHT
    expect(effectiveStripHeight(104, 0)).toBe(104)
    expect(effectiveStripHeight(104, need + 500)).toBe(104)
    expect(effectiveStripHeight(104, need + 90)).toBe(90)
    expect(effectiveStripHeight(104, need)).toBe(72)
    expect(effectiveStripHeight(160, 100)).toBe(72)
  })

  it("T-ui-filmstrip-math-03 [Happy] the window centres the current tile and covers the viewport on both sides", () => {
    // Arrange
    const w = () => 100
    // Act
    const win = stripWindow({ center: 50, range: [0, 1000], viewportW: 1000, widthOf: w })
    // Assert
    expect(win).not.toBeNull()
    expect(win!.first).toBeLessThan(50)
    expect(win!.last).toBeGreaterThan(50)
    expect(win!.offset + (50 - win!.first) * (100 + TILE_GAP)).toBeCloseTo(450) // (1000-100)/2
    expect(win!.last - win!.first + 1).toBeLessThan(40) // 가상화: 1000 중 일부만
    expect(win!.visible).toBeGreaterThanOrEqual(9)
  })

  it("T-ui-filmstrip-math-04 [Boundary] near the ends the window stops at the range; a one-item range is just that item", () => {
    const w = () => 100
    expect(stripWindow({ center: 0, range: [0, 1000], viewportW: 1000, widthOf: w })!.first).toBe(0)
    expect(stripWindow({ center: 999, range: [0, 1000], viewportW: 1000, widthOf: w })!.last).toBe(999)
    const one = stripWindow({ center: 7, range: [7, 8], viewportW: 1000, widthOf: w })!
    expect([one.first, one.last]).toEqual([7, 7])
    // 범위 밖 center 는 범위 안으로 보정
    expect(stripWindow({ center: 5000, range: [100, 200], viewportW: 1000, widthOf: w })!.last).toBe(199)
  })

  it("T-ui-filmstrip-math-05 [Boundary] empty range or unmeasured viewport yields no window", () => {
    expect(stripWindow({ center: 0, range: [0, 0], viewportW: 1000, widthOf: () => 100 })).toBeNull()
    expect(stripWindow({ center: 0, range: [0, 10], viewportW: 0, widthOf: () => 100 })).toBeNull()
  })
})
