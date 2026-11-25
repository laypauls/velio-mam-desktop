// Categories: Happy / Boundary / Validation / Business rule (reserved keys)
import { describe, expect, it } from "vitest"
import { formatShortcut, isReservedShortcut, isValidShortcut, matchesShortcut, shortcutFromEvent } from "./shortcut"

const ev = (key: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({
  key,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
})

describe("shortcut helpers", () => {
  it("T-ui-hidekey-01 [Happy] builds the stored format: fixed modifier order, single letters uppercased, named keys as-is", () => {
    expect(shortcutFromEvent(ev("Escape"))).toBe("Escape")
    expect(shortcutFromEvent(ev("F12"))).toBe("F12")
    expect(shortcutFromEvent(ev("h", { shiftKey: true, ctrlKey: true }))).toBe("Ctrl+Shift+H")
    expect(shortcutFromEvent(ev("x", { shiftKey: true, altKey: true, ctrlKey: true }))).toBe("Ctrl+Alt+Shift+X")
    expect(shortcutFromEvent(ev(" "))).toBe("Space")
  })

  it("T-ui-hidekey-02 [Boundary] pure modifier presses and Meta combos produce no shortcut", () => {
    for (const k of ["Control", "Alt", "Shift", "Meta", "AltGraph"]) expect(shortcutFromEvent(ev(k, { ctrlKey: true }))).toBeNull()
    expect(shortcutFromEvent(ev("h", { metaKey: true }))).toBeNull()
  })

  it("T-ui-hidekey-03 [Happy] matchesShortcut compares exact combos; null (disabled) never matches", () => {
    expect(matchesShortcut(ev("Escape"), "Escape")).toBe(true)
    expect(matchesShortcut(ev("Escape", { shiftKey: true }), "Escape")).toBe(false)
    expect(matchesShortcut(ev("h", { ctrlKey: true, shiftKey: true }), "Ctrl+Shift+H")).toBe(true)
    expect(matchesShortcut(ev("H", { ctrlKey: true, shiftKey: true }), "Ctrl+Shift+H")).toBe(true)
    expect(matchesShortcut(ev("h", { ctrlKey: true }), "Ctrl+Shift+H")).toBe(false)
    expect(matchesShortcut(ev("Escape"), null)).toBe(false)
  })

  it("T-ui-hidekey-04 [Validation] isValidShortcut accepts the stored format and rejects malformed ones", () => {
    for (const ok of ["Escape", "F12", "Ctrl+Shift+H", "Alt+Space"]) expect(isValidShortcut(ok)).toBe(true)
    for (const bad of ["", "Shift+Ctrl+H", "Ctrl+", "a b", "Ctrl+A+B", "+", "ThisKeyNameIsWayTooLong1"]) expect(isValidShortcut(bad)).toBe(false)
  })

  it("T-ui-hidekey-05 [Business rule] keys already used by the app are reserved; Esc, function keys and modified letters are not", () => {
    for (const s of ["Space", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", ",", ".", "M", "Enter", "PageUp", "PageDown", "Home", "End", "0", "3", "5", "Tab", "Ctrl+A", "Ctrl+PageDown", "Shift+ArrowLeft"]) {
      expect(isReservedShortcut(s), s).toBe(true)
    }
    for (const s of ["Escape", "F12", "Ctrl+Shift+H", "Ctrl+M", "Alt+ArrowLeft", "6", "Ctrl+Alt+A"]) expect(isReservedShortcut(s), s).toBe(false)
  })

  it("T-ui-scene-04 [Business rule] scene keys are reserved: C and its Hangul-IME key ㅊ (plain), Ctrl+Arrow (already navigation); modified C stays free", () => {
    for (const s of ["C", "ㅊ", "Ctrl+ArrowLeft", "Ctrl+ArrowRight"]) expect(isReservedShortcut(s), s).toBe(true)
    for (const s of ["Ctrl+C", "Alt+C", "Ctrl+Shift+C", "Alt+ㅊ"]) expect(isReservedShortcut(s), s).toBe(false)
    // 이벤트 → 저장 문자열: 한글 입력 상태의 C 는 "ㅊ" 로 기록되고, 소문자 c 는 "C" 로 정규화된다
    expect(shortcutFromEvent(ev("ㅊ"))).toBe("ㅊ")
    expect(shortcutFromEvent(ev("c"))).toBe("C")
  })

  it("T-ui-hidekey-06 [Happy] formatShortcut shows Escape as Esc", () => {
    expect(formatShortcut("Escape")).toBe("Esc")
    expect(formatShortcut("Ctrl+Escape")).toBe("Ctrl+Esc")
    expect(formatShortcut("F12")).toBe("F12")
  })
})

describe("matchesShortcut physical key (Korean IME)", () => {
  const ime = (key: string, code: string, mods: Partial<{ ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }> = {}) => ({ ...ev(key, mods), code })

  it("T-ui-closekey-01 [Happy] a non-Latin key on the physical Q key matches the letter shortcut Q", () => {
    expect(matchesShortcut(ime("ㅂ", "KeyQ"), "Q")).toBe(true)
    expect(matchesShortcut(ime("ㅃ", "KeyQ", { shiftKey: true }), "Shift+Q")).toBe(true)
    expect(matchesShortcut(ime("ㅂ", "KeyQ", { ctrlKey: true }), "Ctrl+Q")).toBe(true)
  })

  it("T-ui-closekey-02 [Validation] modifiers must still match exactly on the physical path", () => {
    expect(matchesShortcut(ime("ㅂ", "KeyQ", { ctrlKey: true }), "Q")).toBe(false)
    expect(matchesShortcut(ime("ㅂ", "KeyQ"), "Ctrl+Q")).toBe(false)
    expect(matchesShortcut(ime("ㅃ", "KeyQ", { shiftKey: true }), "Q")).toBe(false)
    expect(matchesShortcut(ime("ㅂ", "KeyQ", { metaKey: true }), "Q")).toBe(false)
  })

  it("T-ui-closekey-03 [Boundary] a different physical key, a Latin-letter key, a named-key shortcut or a missing code never matches by code", () => {
    expect(matchesShortcut(ime("ㅂ", "KeyW"), "Q")).toBe(false)
    // AZERTY 처럼 라틴 문자가 찍힌 다른 물리 키는 key 로만 판정한다
    expect(matchesShortcut(ime("a", "KeyQ"), "Q")).toBe(false)
    expect(matchesShortcut(ime("ㅂ", "KeyQ"), "F8")).toBe(false)
    expect(matchesShortcut(ev("ㅂ"), "Q")).toBe(false)
    expect(matchesShortcut(ime("ㅂ", "KeyQ"), null)).toBe(false)
  })
})
