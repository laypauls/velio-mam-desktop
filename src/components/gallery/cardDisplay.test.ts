// Categories: Happy (T-ui-card-01, 02) / Boundary (T-ui-card-03, 04, 08, 09) / Validation (T-ui-card-05, 06, 10) / Business rule (T-ui-card-11)
import { describe, expect, it } from "vitest"
import type { GalleryCardDisplay } from "@/lib/ipc-contract"
import { createMockApi } from "@/lib/api/mock"
import {
  DEFAULT_CARD_DISPLAY,
  CARD_SIZES,
  CARD_SIZE_KEYS,
  PRESETS,
  cardInfoLines,
  gridLayout,
  infoHeight,
  normalize,
  presetOf,
  type CardInfo,
} from "./cardDisplay"

const withInfo = (info: Partial<CardInfo>): GalleryCardDisplay => ({
  ...DEFAULT_CARD_DISPLAY,
  info: { ...PRESETS.off, ...info },
})

describe("presetOf", () => {
  it("T-ui-card-01 [Happy] the three preset combinations map to their names, any other combination is custom", () => {
    // Arrange + Act + Assert
    expect(presetOf({ ...DEFAULT_CARD_DISPLAY, info: PRESETS.off })).toBe("off")
    expect(presetOf({ ...DEFAULT_CARD_DISPLAY, info: PRESETS.compact })).toBe("compact")
    expect(presetOf({ ...DEFAULT_CARD_DISPLAY, info: PRESETS.detailed })).toBe("detailed")
    expect(presetOf(DEFAULT_CARD_DISPLAY)).toBe("detailed")
    expect(presetOf(withInfo({ typeIcon: true, name: true, ext: true, size: true }))).toBe("custom")
    expect(presetOf(withInfo({ name: true }))).toBe("custom")
  })

  it("T-ui-card-02 [Happy] the preset ignores non-info options (date basis, check mode, overlay flags)", () => {
    // Arrange
    const d: GalleryCardDisplay = { ...DEFAULT_CARD_DISPLAY, dateBy: "modified", check: "hidden", favorite: false, duration: false, quickActions: false }
    // Act + Assert
    expect(presetOf(d)).toBe("detailed")
  })
})

describe("cardInfoLines", () => {
  it("T-ui-card-03 [Boundary] 0 / 1 / 2 lines from the item toggles, and infoHeight grows with the line count", () => {
    // Arrange + Act + Assert
    expect(cardInfoLines(withInfo({}), 200)).toBe(0)
    expect(cardInfoLines(withInfo({ typeIcon: true }), 200)).toBe(1)
    expect(cardInfoLines(withInfo({ ext: true }), 200)).toBe(1)
    expect(cardInfoLines(withInfo({ date: true }), 200)).toBe(1)
    expect(cardInfoLines(withInfo({ name: true, size: true }), 200)).toBe(2)
    expect(cardInfoLines(DEFAULT_CARD_DISPLAY, 200)).toBe(2)
    expect(infoHeight(0)).toBe(0)
    expect(infoHeight(1)).toBeGreaterThan(0)
    expect(infoHeight(2)).toBeGreaterThan(infoHeight(1))
  })

  it("T-ui-card-04 [Boundary] below 140px the meta line and the extension chip disappear regardless of options; 140 is still wide", () => {
    // Arrange
    const detailed = DEFAULT_CARD_DISPLAY
    // Act + Assert
    expect(cardInfoLines(detailed, 139)).toBe(1)
    expect(cardInfoLines(detailed, 140)).toBe(2)
    expect(cardInfoLines(withInfo({ ext: true }), 139)).toBe(0) // 확장자만 켠 좁은 카드 → 정보 영역 없음
    expect(cardInfoLines(withInfo({ size: true, date: true }), 139)).toBe(0)
    // 음성은 해상도가 줄을 만들지 못한다. 유형을 생략하면(격자 행 높이) 최대값
    const resOnly = withInfo({ resolution: true })
    expect(cardInfoLines(resOnly, 200, "audio")).toBe(0)
    expect(cardInfoLines(resOnly, 200, "video")).toBe(1)
    expect(cardInfoLines(resOnly, 200)).toBe(1)
  })
})

describe("normalize", () => {
  it("T-ui-card-05 [Validation] non-objects and arrays fall back to the defaults (as a copy)", () => {
    // Arrange + Act + Assert
    for (const bad of [null, undefined, 3, "x", [], true]) {
      const v = normalize(bad)
      expect(v).toEqual(DEFAULT_CARD_DISPLAY)
      expect(v).not.toBe(DEFAULT_CARD_DISPLAY)
      expect(v.info).not.toBe(DEFAULT_CARD_DISPLAY.info)
    }
  })

  it("T-ui-card-06 [Validation] wrong-typed fields get field-level defaults, unknown fields are dropped, valid fields are kept", () => {
    // Arrange
    const raw = {
      info: { name: false, ext: "yes", size: 0, bogus: true },
      dateBy: "modified",
      check: "sometimes",
      favorite: false,
      duration: "no",
      future: { a: 1 },
    }
    // Act
    const v = normalize(raw)
    // Assert
    expect(v).toEqual({
      info: { ...DEFAULT_CARD_DISPLAY.info, name: false },
      dateBy: "modified",
      check: DEFAULT_CARD_DISPLAY.check,
      favorite: false,
      rating: true,
      duration: true,
      quickActions: true,
      cardSize: "m",
    })
    expect(v).not.toHaveProperty("future")
    expect(v.info).not.toHaveProperty("bogus")
    expect(normalize({ info: "broken" })).toEqual(DEFAULT_CARD_DISPLAY)
  })

  it("T-ui-card-07 [Happy] the mock settings default equals the frontend default (the contract defaults stay in sync)", async () => {
    // Arrange + Act
    const settings = await createMockApi({ count: 1 }).getSettings()
    // Assert
    expect(settings.galleryCardDisplay).toEqual(DEFAULT_CARD_DISPLAY)
    expect(normalize(settings.galleryCardDisplay)).toEqual(settings.galleryCardDisplay)
  })
})

describe("gridLayout / card sizes", () => {
  const GAP = 8
  const total = (g: { cols: number; cell: number }) => g.cols * g.cell + (g.cols - 1) * GAP

  it("T-ui-card-08 [Boundary] inner 0 or negative gives one zero-width column; narrower than the target gives one column that fits", () => {
    // Arrange + Act + Assert
    expect(gridLayout(0, 180, GAP)).toEqual({ cols: 1, cell: 0 })
    expect(gridLayout(-5, 180, GAP)).toEqual({ cols: 1, cell: 0 })
    expect(gridLayout(Number.NaN, 180, GAP)).toEqual({ cols: 1, cell: 0 })
    expect(gridLayout(126, 180, GAP)).toEqual({ cols: 1, cell: 126 })
    expect(gridLayout(400, 300, GAP)).toEqual({ cols: 1, cell: 400 }) // 1열은 상한 없이 채운다
  })

  it("T-ui-card-09 [Boundary] exactly n cells keeps the target; n+0.5 cells rounds to the nearest column count", () => {
    // Arrange: 5칸 = 5*180 + 4*8
    const five = 5 * 180 + 4 * GAP
    // Act + Assert
    expect(gridLayout(five, 180, GAP)).toEqual({ cols: 5, cell: 180 })
    const half = gridLayout(five + (180 + GAP) * 0.5 - 0.01, 180, GAP) // 5.5칸 바로 아래 -> 5열
    expect(half.cols).toBe(5)
    expect(gridLayout(five + (180 + GAP) * 0.5, 180, GAP).cols).toBe(6) // 5.5칸 -> 6열
    expect(half.cell).toBeGreaterThan(180)
  })

  it("T-ui-card-10 [Validation] no leftover on the right for any width and size, and the cell stays within about +-1/(2*cols) of the target when cols >= 2", () => {
    // Arrange + Act + Assert
    for (const k of CARD_SIZE_KEYS) {
      for (let inner = 1; inner <= 2600; inner += 7) {
        const g = gridLayout(inner, CARD_SIZES[k], GAP)
        expect(Math.abs(total(g) - inner)).toBeLessThan(1e-9)
        expect(g.cell).toBeGreaterThan(0)
        if (g.cols >= 2) expect(Math.abs(g.cell - CARD_SIZES[k])).toBeLessThanOrEqual((CARD_SIZES[k] + GAP) / (2 * g.cols) + 1e-9)
      }
    }
  })

  it("T-ui-card-11 [Business rule] sizes are ordered, and for the same width a larger size never gives a smaller card (inner 1000..2560)", () => {
    // Arrange
    const targets = CARD_SIZE_KEYS.map((k) => CARD_SIZES[k])
    expect([...targets].sort((a, b) => a - b)).toEqual(targets)
    expect(CARD_SIZES.m).toBe(180)
    // Act + Assert
    for (let inner = 1000; inner <= 2560; inner++) {
      const cells = targets.map((t) => gridLayout(inner, t, GAP).cell)
      for (let i = 1; i < cells.length; i++) expect(cells[i]).toBeGreaterThanOrEqual(cells[i - 1])
    }
  })

  it("T-ui-card-12 [Validation] normalize: missing or invalid cardSize falls back to m, valid ones are kept", () => {
    // Arrange + Act + Assert
    expect(normalize({}).cardSize).toBe("m")
    expect(normalize({ cardSize: "huge" }).cardSize).toBe("m")
    expect(normalize({ cardSize: 180 }).cardSize).toBe("m")
    expect(normalize({ cardSize: null }).cardSize).toBe("m")
    for (const k of CARD_SIZE_KEYS) expect(normalize({ cardSize: k }).cardSize).toBe(k)
    expect(DEFAULT_CARD_DISPLAY.cardSize).toBe("m")
  })
})
