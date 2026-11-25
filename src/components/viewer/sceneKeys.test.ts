// Categories: Business rule (T-ui-scene-02 sceneNav) / Validation (T-ui-scene-03 key judgement) / Boundary (T-ui-scene-02 half frame table)
import { describe, expect, it } from "vitest"
import { frameMs, halfFrameMs, sceneKeyAction, sceneNav } from "./sceneKeys"

type Mods = { ctrl?: boolean; alt?: boolean; shift?: boolean; meta?: boolean }
const ev = (key: string, code: string, m: Mods = {}) => ({ key, code, ctrlKey: !!m.ctrl, altKey: !!m.alt, shiftKey: !!m.shift, metaKey: !!m.meta })

describe("half frame / frame length", () => {
  it("T-ui-scene-02a [Boundary] half frame = max(1, round(500/fps)), 20ms when fps is missing or invalid; frame = max(1, round(1000/fps)) else 40ms", () => {
    // Arrange / Act / Assert: 서버 `half_frame_ms` 와 같은 표
    expect(halfFrameMs(25)).toBe(20)
    expect(halfFrameMs(29.97)).toBe(17)
    expect(halfFrameMs(59.94)).toBe(8)
    expect(halfFrameMs(1000)).toBe(1)
    expect(halfFrameMs(100_000)).toBe(1)
    for (const bad of [null, 0, -5, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(halfFrameMs(bad)).toBe(20)
      expect(frameMs(bad)).toBe(40)
    }
    expect(frameMs(25)).toBe(40)
    expect(frameMs(29.97)).toBe(33)
  })
})

describe("sceneNav (fps 25: half frame 20ms, frame 40ms)", () => {
  const cuts = [10_000, 20_000, 30_000]

  it("T-ui-scene-02b [Business rule] next goes to the first cut after the base, and the target is cut + half frame", () => {
    expect(sceneNav(cuts, 0, 1, 25)).toBe(10_020)
    expect(sceneNav(cuts, 15_000, 1, 25)).toBe(20_020)
  })

  it("T-ui-scene-02c [Business rule] rapid presses: using the previous target as base walks forward one cut at a time (never returns to the same cut)", () => {
    // Arrange: 연타 — 기준은 직전 이동 목표
    let base = 0
    const visited: number[] = []
    // Act
    for (let i = 0; i < 3; i++) {
      const to = sceneNav(cuts, base, 1, 25)
      if (to === null) break
      visited.push(to)
      base = to
    }
    // Assert
    expect(visited).toEqual([10_020, 20_020, 30_020])
    expect(sceneNav(cuts, base, 1, 25)).toBeNull()
  })

  it("T-ui-scene-02d [Business rule] prev walks backward and excludes the cut we are standing on (base - one frame margin)", () => {
    expect(sceneNav(cuts, 30_020, -1, 25)).toBe(20_020)
    expect(sceneNav(cuts, 20_020, -1, 25)).toBe(10_020)
    expect(sceneNav(cuts, 10_020, -1, 25)).toBeNull()
    // 컷 직후 한 프레임 안(기준 − 40ms 이내)에서는 같은 컷으로 되돌아가지 않는다
    expect(sceneNav(cuts, 10_050, -1, 25)).toBeNull()
    // 한 프레임 넘게 지나면 방금 지난 컷으로 돌아간다
    expect(sceneNav(cuts, 10_061, -1, 25)).toBe(10_020)
  })

  it("T-ui-scene-02e [Boundary] no cuts / before the first / after the last → null; next skips a cut whose target is within 1ms of the base", () => {
    expect(sceneNav([], 5000, 1, 25)).toBeNull()
    expect(sceneNav([], 5000, -1, 25)).toBeNull()
    expect(sceneNav(cuts, 5000, -1, 25)).toBeNull()
    expect(sceneNav(cuts, 30_100, 1, 25)).toBeNull()
    // base 가 컷 + 반 프레임 − 1ms 이내면(= x+hf > base+1 거짓) 그 컷은 이미 도착한 것으로 본다
    expect(sceneNav(cuts, 10_019, 1, 25)).toBe(20_020)
    expect(sceneNav(cuts, 10_018, 1, 25)).toBe(10_020)
  })

  it("T-ui-scene-02f [Boundary] with an unknown fps the 20ms / 40ms fallbacks apply", () => {
    expect(sceneNav(cuts, 0, 1, null)).toBe(10_020)
    expect(sceneNav(cuts, 20_020, -1, null)).toBe(10_020)
  })
})

describe("sceneKeyAction", () => {
  it("T-ui-scene-03a [Happy] Ctrl+ArrowLeft/Right move, bare KeyC adds — also when the IME reports Hangul in key (judged by code)", () => {
    expect(sceneKeyAction(ev("ArrowLeft", "ArrowLeft", { ctrl: true }))).toBe("prev")
    expect(sceneKeyAction(ev("ArrowRight", "ArrowRight", { ctrl: true }))).toBe("next")
    expect(sceneKeyAction(ev("c", "KeyC"))).toBe("add")
    expect(sceneKeyAction(ev("ㅊ", "KeyC"))).toBe("add")
  })

  it("T-ui-scene-03b [Validation] exact modifiers: Ctrl+C (copy), Ctrl+Shift+Arrow (gallery), Alt+Arrow (history), Shift+C, Meta combos and plain arrows are not scene keys", () => {
    const rejected: [string, string, Mods][] = [
      ["c", "KeyC", { ctrl: true }],
      ["ArrowLeft", "ArrowLeft", { ctrl: true, shift: true }],
      ["ArrowRight", "ArrowRight", { ctrl: true, alt: true }],
      ["ArrowLeft", "ArrowLeft", { alt: true }],
      ["ArrowLeft", "ArrowLeft", {}],
      ["ArrowRight", "ArrowRight", { shift: true }],
      ["C", "KeyC", { shift: true }],
      ["c", "KeyC", { alt: true }],
      ["c", "KeyC", { meta: true }],
      ["ArrowLeft", "ArrowLeft", { ctrl: true, meta: true }],
      ["ArrowUp", "ArrowUp", { ctrl: true }],
      ["x", "KeyX", {}],
    ]
    for (const [key, code, m] of rejected) expect(sceneKeyAction(ev(key, code, m)), `${code} ${JSON.stringify(m)}`).toBeNull()
  })
})
