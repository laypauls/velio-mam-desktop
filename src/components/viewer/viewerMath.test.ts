// Categories: Happy (T-ui-viewer-20, -22) / Boundary (T-ui-viewer-21, -23, -24) / Validation (T-ui-viewer-25)
import { describe, expect, it } from "vitest"
import { clampScale, clampTime, fitScale, MAX_SCALE, MIN_SCALE, stepFrame, zoomAround } from "./viewerMath"

describe("viewerMath", () => {
  it("T-ui-viewer-20 [Happy] stepFrame moves by exactly 1/fps in either direction", () => {
    // Arrange + Act + Assert
    expect(stepFrame(10, 25, 1, 100)).toBeCloseTo(10.04, 10)
    expect(stepFrame(10, 25, -1, 100)).toBeCloseTo(9.96, 10)
  })

  it("T-ui-viewer-21 [Boundary] stepFrame falls back to 30 fps for missing/zero/negative fps and clamps to the range", () => {
    // Arrange + Act + Assert
    expect(stepFrame(1, null, 1, 100)).toBeCloseTo(1 + 1 / 30, 10)
    expect(stepFrame(1, 0, 1, 100)).toBeCloseTo(1 + 1 / 30, 10)
    expect(stepFrame(1, -5, -1, 100)).toBeCloseTo(1 - 1 / 30, 10)
    expect(stepFrame(0, 30, -1, 100)).toBe(0)
    expect(stepFrame(99.99, 30, 1, 100)).toBe(100)
    expect(stepFrame(5, 30, 1, Number.NaN)).toBeCloseTo(5 + 1 / 30, 10)
  })

  it("T-ui-viewer-22 [Happy] fitScale shrinks to the box, never upscales, and swaps sides when rotated", () => {
    // Arrange
    const box = { width: 500, height: 400 }
    const natural = { width: 1000, height: 500 }
    // Act + Assert
    expect(fitScale(box, natural, 0)).toBe(0.5)
    expect(fitScale(box, natural, 90)).toBe(0.4)
    expect(fitScale(box, { width: 100, height: 50 }, 0)).toBe(1)
  })

  it("T-ui-viewer-23 [Boundary] fitScale is 1 while the box or the image size is unknown", () => {
    // Arrange + Act + Assert
    expect(fitScale({ width: 0, height: 0 }, { width: 100, height: 100 }, 0)).toBe(1)
    expect(fitScale({ width: 100, height: 100 }, null, 0)).toBe(1)
    expect(fitScale({ width: 100, height: 100 }, { width: 0, height: 0 }, 0)).toBe(1)
  })

  it("T-ui-viewer-24 [Boundary] scale is clamped to [MIN_SCALE, MAX_SCALE] and clampTime to [0, duration]", () => {
    // Arrange + Act + Assert
    expect(clampScale(1000)).toBe(MAX_SCALE)
    expect(clampScale(0)).toBe(MIN_SCALE)
    expect(clampTime(-3, 10)).toBe(0)
    expect(clampTime(30, 10)).toBe(10)
  })

  it("T-ui-viewer-25 [Validation] zoomAround keeps the point under the cursor fixed", () => {
    // Arrange: 커서가 중심에서 (100, 50) 떨어진 곳, 이동량 (10, -20), 2배 확대
    const before = { scale: 1, tx: 10, ty: -20 }
    const [px, py] = [100, 50]
    // Act
    const after = zoomAround(before, 2, px, py)
    // Assert: 화면 좌표 = 이동량 + 배율 * 이미지 좌표 — 커서 아래 이미지 좌표가 같아야 한다
    const imageX = (px - before.tx) / before.scale
    const imageY = (py - before.ty) / before.scale
    expect(after.tx + after.scale * imageX).toBeCloseTo(px, 10)
    expect(after.ty + after.scale * imageY).toBeCloseTo(py, 10)
    expect(after.scale).toBe(2)
  })
})
