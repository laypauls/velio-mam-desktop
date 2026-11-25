// Categories: Happy (T-ui-rangekeys-01, 02) / Validation (T-ui-rangekeys-03 modifiers, IME) / Business rule (T-ui-rangekeys-04 conflicts, 05 no clash with existing keys)
import { describe, expect, it } from "vitest"
import { playerKeyAction } from "./playerKeys"
import { RANGE_KEY_PRESETS, playRangeKeyAction, presetConflict, rangeKeyLabel, type RangeKeyActionType } from "./playRangeKeys"

type Mods = { ctrl?: boolean; alt?: boolean; shift?: boolean; meta?: boolean }
const ev = (code: string, key = "", m: Mods = {}) => ({ code, key, ctrlKey: !!m.ctrl, altKey: !!m.alt, shiftKey: !!m.shift, metaKey: !!m.meta })

describe("range key presets", () => {
  const brackets: [string, Mods, RangeKeyActionType][] = [
    ["BracketLeft", {}, "markIn"],
    ["BracketRight", {}, "markOut"],
    ["BracketLeft", { shift: true }, "gotoPrev"],
    ["BracketRight", { shift: true }, "gotoNext"],
    ["Backslash", {}, "selectAtPlayhead"],
    ["BracketLeft", { alt: true }, "clearIn"],
    ["BracketRight", { alt: true }, "clearOut"],
    ["KeyX", { ctrl: true, shift: true }, "deleteAll"],
    ["Delete", {}, "deleteSelected"],
  ]
  const premiere: [string, Mods, RangeKeyActionType][] = [
    ["KeyI", {}, "markIn"],
    ["KeyO", {}, "markOut"],
    ["KeyI", { shift: true }, "gotoPrev"],
    ["KeyO", { shift: true }, "gotoNext"],
    ["KeyX", {}, "selectAtPlayhead"],
    ["KeyI", { ctrl: true, shift: true }, "clearIn"],
    ["KeyO", { ctrl: true, shift: true }, "clearOut"],
    ["KeyX", { ctrl: true, shift: true }, "deleteAll"],
    ["Delete", {}, "deleteSelected"],
  ]

  it.each(brackets)("T-ui-rangekeys-01 [Happy] brackets preset: %s %j → %s", (code, mods, action) => {
    // Arrange + Act + Assert
    expect(playRangeKeyAction(ev(code, "", mods), "brackets")).toEqual({ type: action })
  })

  it.each(premiere)("T-ui-rangekeys-02 [Happy] premiere preset: %s %j → %s", (code, mods, action) => {
    // Arrange + Act + Assert
    expect(playRangeKeyAction(ev(code, "", mods), "premiere")).toEqual({ type: action })
  })

  it("T-ui-rangekeys-03 [Validation] Korean IME: the character comes out as 'ㅑ' / 'ㅐ' / 'ㅌ' but the physical code still maps; exact modifiers only; Meta never", () => {
    // Arrange + Act + Assert: IME
    expect(playRangeKeyAction(ev("KeyI", "ㅑ"), "premiere")).toEqual({ type: "markIn" })
    expect(playRangeKeyAction(ev("KeyO", "ㅐ"), "premiere")).toEqual({ type: "markOut" })
    expect(playRangeKeyAction(ev("KeyX", "ㅌ"), "premiere")).toEqual({ type: "selectAtPlayhead" })
    // 프리셋이 다르면 같은 물리 키라도 동작하지 않는다
    expect(playRangeKeyAction(ev("KeyI", "ㅑ"), "brackets")).toBeNull()
    expect(playRangeKeyAction(ev("BracketLeft", "["), "premiere")).toBeNull()
    // 수정자는 정확히 일치: 괄호 + Ctrl, I + Alt, Shift 없는 Ctrl+X 는 어느 쪽도 아님
    expect(playRangeKeyAction(ev("BracketLeft", "[", { ctrl: true }), "brackets")).toBeNull()
    expect(playRangeKeyAction(ev("KeyI", "i", { alt: true }), "premiere")).toBeNull()
    expect(playRangeKeyAction(ev("KeyX", "x", { ctrl: true }), "premiere")).toBeNull()
    expect(playRangeKeyAction(ev("Delete", "Delete", { shift: true }), "brackets")).toBeNull()
    // Meta(Win) 조합은 항상 제외
    expect(playRangeKeyAction(ev("KeyI", "i", { meta: true }), "premiere")).toBeNull()
  })

  it("T-ui-rangekeys-04 [Business rule] labels follow the preset; presetConflict detects hide-shortcut clashes by physical key (incl. shifted characters), ignores unrelated and null", () => {
    // Arrange + Act + Assert: 표기
    expect(rangeKeyLabel("brackets", "gotoPrev")).toBe("Shift+[")
    expect(rangeKeyLabel("premiere", "gotoPrev")).toBe("Shift+I")
    // 충돌
    expect(presetConflict("premiere", "I")).toBe("I")
    expect(presetConflict("premiere", "Shift+O")).toBe("Shift+O")
    expect(presetConflict("brackets", "[")).toBe("[")
    expect(presetConflict("brackets", "Shift+{")).toBe("Shift+[") // Shift+[ 로 기록되면 key 는 "{"
    expect(presetConflict("brackets", "Alt+]")).toBe("Alt+]")
    expect(presetConflict("brackets", "Delete")).toBe("Delete")
    expect(presetConflict("brackets", "Ctrl+Shift+X")).toBe("Ctrl+Shift+X")
    expect(presetConflict("premiere", "X")).toBe("X")
    // 겹치지 않음
    expect(presetConflict("brackets", "I")).toBeNull()
    expect(presetConflict("premiere", "[")).toBeNull()
    expect(presetConflict("premiere", "Ctrl+I")).toBeNull()
    expect(presetConflict("brackets", "Escape")).toBeNull()
    expect(presetConflict("brackets", null)).toBeNull()
  })

  it("T-ui-rangekeys-05 [Business rule] neither preset reuses a key the base player shortcuts already take, and bindings inside a preset are unique", () => {
    // Arrange
    for (const preset of ["brackets", "premiere"] as const) {
      const seen = new Set<string>()
      for (const b of RANGE_KEY_PRESETS[preset]) {
        // Act
        const id = `${b.code}|${!!b.ctrl}|${!!b.alt}|${!!b.shift}`
        // Assert: 프리셋 안에서 유일, 기본 키(KeyM 포함)와 겹치지 않음
        expect(seen.has(id)).toBe(false)
        seen.add(id)
        expect(playerKeyAction(ev(b.code, b.label.split("+").pop() ?? "", { ctrl: b.ctrl, alt: b.alt, shift: b.shift }), true)).toBeNull()
        expect(b.code).not.toBe("KeyM")
      }
    }
  })
})
