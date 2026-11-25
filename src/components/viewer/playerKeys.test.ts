// Categories: Happy / Boundary (modifiers, audio) / Business rule (volume clamp)
import { describe, expect, it } from "vitest"
import { nextVolume, playbackKeyAction, playerKeyAction } from "./playerKeys"

const ev = (key: string, mods: Partial<{ code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({
  key,
  code: key.length === 1 && /[a-z]/i.test(key) ? `Key${key.toUpperCase()}` : key,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
})

describe("playerKeyAction", () => {
  it("T-ui-playerkeys-01 [Happy] maps Space, arrows, comma/period and M", () => {
    expect(playerKeyAction(ev(" "), true)).toEqual({ type: "togglePlay" })
    expect(playerKeyAction(ev("ArrowRight"), true)).toEqual({ type: "seekBy", delta: 5 })
    expect(playerKeyAction(ev("ArrowLeft"), true)).toEqual({ type: "seekBy", delta: -5 })
    expect(playerKeyAction(ev("ArrowUp"), true)).toEqual({ type: "volumeBy", delta: 0.05 })
    expect(playerKeyAction(ev("ArrowDown"), true)).toEqual({ type: "volumeBy", delta: -0.05 })
    expect(playerKeyAction(ev("."), true)).toEqual({ type: "step", dir: 1 })
    expect(playerKeyAction(ev(","), true)).toEqual({ type: "step", dir: -1 })
    expect(playerKeyAction(ev("m"), true)).toEqual({ type: "toggleMute" })
    expect(playerKeyAction(ev("M"), true)).toEqual({ type: "toggleMute" })
  })

  it("T-ui-playerkeys-02 [Boundary] frame step is video only; any modifier or unknown key maps to nothing", () => {
    expect(playerKeyAction(ev("."), false)).toBeNull()
    expect(playerKeyAction(ev(","), false)).toBeNull()
    expect(playerKeyAction(ev("m"), false)).toEqual({ type: "toggleMute" })
    for (const mod of ["ctrlKey", "altKey", "shiftKey", "metaKey"] as const) expect(playerKeyAction(ev("ArrowRight", { [mod]: true }), true)).toBeNull()
    expect(playerKeyAction(ev("Enter"), true)).toBeNull()
    expect(playerKeyAction(ev("Escape"), true)).toBeNull()
  })

  it("T-ui-playerkeys-03 [Boundary] nextVolume clamps to 0..1 without float drift", () => {
    expect(nextVolume(1, 0.05)).toBe(1)
    expect(nextVolume(0, -0.05)).toBe(0)
    expect(nextVolume(0.7, 0.05)).toBe(0.75)
    expect(nextVolume(0.15, -0.05)).toBe(0.1)
  })

  it("T-ui-playerkeys-04 [Happy] Korean IME: key is a jamo but the physical code KeyM still mutes", () => {
    expect(playerKeyAction(ev("ㅡ", { code: "KeyM" }), true)).toEqual({ type: "toggleMute" })
    expect(playerKeyAction(ev("ㅡ", { code: "KeyN" }), true)).toBeNull()
  })

  it("T-ui-playerkeys-05 [Validation] Shift+M is not mute", () => {
    expect(playerKeyAction(ev("M", { shiftKey: true }), true)).toBeNull()
  })

  it("T-ui-playerkeys-06 [Happy] playbackKeyAction maps Shift+KeyN/KeyP by physical key (IME safe); needs Shift, rejects Ctrl", () => {
    expect(playbackKeyAction(ev("ㅜ", { code: "KeyN", shiftKey: true }))).toBe("next")
    expect(playbackKeyAction(ev("ㅔ", { code: "KeyP", shiftKey: true }))).toBe("prev")
    expect(playbackKeyAction(ev("N", { shiftKey: true, ctrlKey: true }))).toBeNull()
    expect(playbackKeyAction(ev("n"))).toBeNull()
    expect(playbackKeyAction(ev("X", { shiftKey: true }))).toBeNull()
  })
})

describe("Shift = system volume", () => {
  it("T-ui-sysvol-key-01 [Happy] Shift+ArrowUp / ArrowDown map to systemVolumeBy ±5%, plain arrows stay app volume", () => {
    expect(playerKeyAction(ev("ArrowUp", { shiftKey: true }), true)).toEqual({ type: "systemVolumeBy", delta: 0.05 })
    expect(playerKeyAction(ev("ArrowDown", { shiftKey: true }), false)).toEqual({ type: "systemVolumeBy", delta: -0.05 })
    expect(playerKeyAction(ev("ArrowUp"), true)).toEqual({ type: "volumeBy", delta: 0.05 })
  })

  it("T-ui-sysvol-key-02 [Boundary] Shift with other keys stays unmapped (Shift+N/P go to playbackKeyAction; range keys Shift+[ ] / Shift+I/O are not stolen); Ctrl/Alt/Meta+Shift+Arrow is null", () => {
    expect(playerKeyAction(ev("N", { shiftKey: true }), true)).toBeNull()
    expect(playerKeyAction(ev("P", { shiftKey: true }), true)).toBeNull()
    expect(playerKeyAction(ev("{", { code: "BracketLeft", shiftKey: true }), true)).toBeNull()
    expect(playerKeyAction(ev("I", { shiftKey: true }), true)).toBeNull()
    expect(playerKeyAction(ev("ArrowLeft", { shiftKey: true }), true)).toBeNull()
    for (const mod of ["ctrlKey", "altKey", "metaKey"] as const) expect(playerKeyAction(ev("ArrowUp", { shiftKey: true, [mod]: true }), true)).toBeNull()
    expect(playbackKeyAction(ev("N", { shiftKey: true }))).toBe("next")
  })
})

describe("fullscreen key", () => {
  it("T-ui-fullscreen-14 [Happy] F (physical key, also under a Korean IME) maps to toggleFullscreen for video and audio", () => {
    expect(playerKeyAction(ev("f"), true)).toEqual({ type: "toggleFullscreen" })
    expect(playerKeyAction(ev("f"), false)).toEqual({ type: "toggleFullscreen" })
    expect(playerKeyAction(ev("ㄹ", { code: "KeyF" }), true)).toEqual({ type: "toggleFullscreen" })
  })

  it("T-ui-fullscreen-15 [Boundary] F with Ctrl/Alt/Meta/Shift maps to nothing (Ctrl+F stays find, Shift+F is not ours)", () => {
    for (const mod of ["ctrlKey", "altKey", "metaKey", "shiftKey"] as const) expect(playerKeyAction(ev("f", { [mod]: true }), true)).toBeNull()
  })
})
