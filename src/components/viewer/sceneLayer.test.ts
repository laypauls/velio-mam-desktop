// Categories: Happy (T-ui-track-10) / Boundary (T-ui-track-11, 12) / Business rule (T-ui-track-13)
import { describe, expect, it } from "vitest"
import type { SceneCut } from "@/lib/ipc-contract"
import { CUT_BUCKET_PX, buildCutPaths, nearestCut, trianglePath } from "./sceneLayer"

const auto = (ms: number): SceneCut => ({ ms, source: "auto" })
const user = (ms: number): SceneCut => ({ ms, source: "user" })
const count = (d: string) => (d.match(/M/g) ?? []).length

describe("buildCutPaths (inverted triangles)", () => {
  it("T-ui-track-10 [Happy] auto cuts are 10x7 triangles, user cuts 12x9, centred on x with the apex pointing down", () => {
    // Arrange: 100초, 트랙 1000px → 1px = 100ms
    // Act
    const p = buildCutPaths([auto(10_000), user(20_000)], 100_000, 1000)
    // Assert
    expect(p.auto).toBe("M95 0h10l-5 7z")
    expect(p.user).toBe("M194 0h12l-6 9z")
    expect(trianglePath(0, "auto", 2)).toBe("M-10 0h20l-10 14z")
  })

  it("T-ui-track-11 [Boundary] cuts in the same 4px bucket merge per source; the next bucket and the other source stay separate", () => {
    // Arrange: x = 100, 100, 103 (같은 칸) / 104 (다음 칸)
    const cuts = [auto(10_000), auto(10_010), auto(10_300), auto(10_400), user(10_000)]
    // Act
    const p = buildCutPaths(cuts, 100_000, 1000)
    // Assert
    expect(CUT_BUCKET_PX).toBe(4)
    expect(count(p.auto)).toBe(2)
    expect(count(p.user)).toBe(1)
  })

  it("T-ui-track-12 [Boundary] thousands of cuts yield at most width/4 + 1 triangles per source; empty input, zero duration and zero width yield empty paths; out-of-range ms is clamped", () => {
    // Arrange
    const dense = Array.from({ length: 5000 }, (_, i) => auto(i * 20))
    // Act + Assert
    expect(count(buildCutPaths(dense, 100_000, 1000).auto)).toBeLessThanOrEqual(251)
    expect(buildCutPaths([], 100_000, 1000)).toEqual({ auto: "", user: "" })
    expect(buildCutPaths([auto(10)], 0, 1000)).toEqual({ auto: "", user: "" })
    expect(buildCutPaths([auto(10)], 100_000, 0)).toEqual({ auto: "", user: "" })
    expect(buildCutPaths([auto(-500), auto(999_999)], 100_000, 1000).auto).toBe("M-5 0h10l-5 7zM995 0h10l-5 7z")
  })
})

describe("nearestCut", () => {
  it("T-ui-track-13 [Business rule] picks the closest cut within ±4px, none farther, and the closer one when two are in range", () => {
    // Arrange: 1px = 100ms
    const cuts = [auto(10_000), user(10_600), auto(50_000)]
    // Act + Assert
    expect(nearestCut(cuts, 100_000, 1000, 96)).toEqual(auto(10_000))
    expect(nearestCut(cuts, 100_000, 1000, 95)).toBeNull()
    expect(nearestCut(cuts, 100_000, 1000, 104)).toEqual(user(10_600))
    expect(nearestCut([], 100_000, 1000, 100)).toBeNull()
  })
})
