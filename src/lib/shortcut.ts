/**
 * 단축키 표기 — 설정 `hideShortcut` 과 같은 형식: 수정자 `Ctrl+` `Alt+` `Shift+`(이 순서) + 키.
 * 키는 `KeyboardEvent.key` 이고 한 글자는 대문자로 정규화한다(예: "Escape", "F12", "Ctrl+Shift+H").
 */

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "AltGraph", "OS", "Dead", "Process", "Unidentified"])

type KeyLike = Pick<KeyboardEvent, "key" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey"> & { code?: string }

function modifierPrefix(e: KeyLike): string {
  return `${e.ctrlKey ? "Ctrl+" : ""}${e.altKey ? "Alt+" : ""}${e.shiftKey ? "Shift+" : ""}`
}

/** 눌린 키 조합을 설정 형식 문자열로 바꾼다. 수정자만 눌렀거나 Meta(Win) 조합이면 null */
export function shortcutFromEvent(e: KeyLike): string | null {
  if (MODIFIER_KEYS.has(e.key) || e.metaKey) return null
  const key = e.key === " " ? "Space" : e.key.length === 1 ? e.key.toUpperCase() : e.key
  return `${modifierPrefix(e)}${key}`
}

/**
 * 이벤트가 저장된 단축키(null = 사용 안 함)와 같은 조합인가.
 * 키가 A–Z 한 글자인 단축키는 한글 입력 상태처럼 `key` 가 라틴 문자가 아닐 때(예: "ㅂ") 물리 키(`code === "KeyQ"`)로도 일치시킨다 —
 * 수정자는 어느 경로든 정확히 일치해야 한다. 라틴 문자가 찍힌 키는 `key` 로만 판정한다(AZERTY·Dvorak 에서 다른 키가 걸리지 않게).
 */
export function matchesShortcut(e: KeyLike, shortcut: string | null): boolean {
  if (shortcut === null) return false
  if (shortcutFromEvent(e) === shortcut) return true
  if (e.metaKey || !e.code || /^[A-Za-z]$/.test(e.key)) return false
  const m = /^((?:Ctrl\+)?(?:Alt\+)?(?:Shift\+)?)([A-Z])$/.exec(shortcut)
  return m !== null && e.code === `Key${m[2]}` && modifierPrefix(e) === m[1]
}

/** 저장 가능한 형식인가 — 키 토큰은 "+"·공백 없이 1~20자 */
export function isValidShortcut(s: string): boolean {
  const m = /^(Ctrl\+)?(Alt\+)?(Shift\+)?([^+\s]{1,20})$/.exec(s)
  return m !== null
}

/** 수정자(Ctrl/Shift)와 함께여도 갤러리 이동·선택에 쓰이는 키 */
const NAVIGATION_KEYS = new Set(["Space", "Enter", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"])
/** 수정자 없이 쓰이는 키 — 플레이어(, . M · 장면전환점 추가 C) · 별점(0~5). 장면 추가는 물리 키(KeyC)로 판정하므로 한글 입력 상태의 "ㅊ" 도 예약한다 */
const PLAIN_KEYS = new Set([",", ".", "M", "C", "ㅊ", "0", "1", "2", "3", "4", "5"])

/** 이미 앱의 다른 기능에 쓰이는 조합이면 true — 지정하면 그 기능이 사라지므로 거부한다 */
export function isReservedShortcut(s: string): boolean {
  const parts = s.split("+")
  const key = parts[parts.length - 1] === "" ? "+" : parts[parts.length - 1]
  const mods = new Set(parts.slice(0, -1))
  if (key === "Tab") return true
  if (s === "Ctrl+A") return true
  // 연속 재생 다음/이전
  if (s === "Shift+N" || s === "Shift+P") return true
  const onlyCtrlShift = [...mods].every((m) => m === "Ctrl" || m === "Shift")
  if (NAVIGATION_KEYS.has(key) && onlyCtrlShift) return true
  return mods.size === 0 && PLAIN_KEYS.has(key)
}

/** 표시용 — "Escape" 는 "Esc" 로, "Space" 등은 그대로 */
export function formatShortcut(s: string): string {
  return s === "Escape" ? "Esc" : s.replace(/\+Escape$/, "+Esc")
}
