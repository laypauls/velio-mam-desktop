import type { PlayRangeKeys } from "@/lib/ipc-contract"

/** 구간 키 동작 — 설정 프리셋(괄호 / Premiere)이 같은 동작 집합에 다른 키를 대응시킨다 */
export type RangeKeyActionType =
  | "markIn"
  | "markOut"
  | "gotoPrev"
  | "gotoNext"
  | "selectAtPlayhead"
  | "clearIn"
  | "clearOut"
  | "deleteAll"
  | "deleteSelected"

export interface RangeKeyAction {
  type: RangeKeyActionType
}

/** usePlayerKeys 가 구간 키를 넘기는 대상 — 컨트롤러가 채운다 */
export interface RangeKeyHandlers {
  preset: PlayRangeKeys
  /** 선택된 구간이 있는가 — Delete 는 선택이 있을 때만 플레이어가 가져간다 */
  hasSelection: boolean
  run: (action: RangeKeyActionType) => void
}

/** 물리 키(`KeyboardEvent.code`) + 정확한 수정자 조합 — 한글 입력 상태에서 `key` 가 "ㅑ" 가 되어도 동작한다 */
export interface RangeBinding {
  action: RangeKeyActionType
  code: string
  ctrl?: boolean
  alt?: boolean
  shift?: boolean
  /** 화면 표기 — 메뉴·설정 안내 */
  label: string
}

const BRACKETS: readonly RangeBinding[] = [
  { action: "markIn", code: "BracketLeft", label: "[" },
  { action: "markOut", code: "BracketRight", label: "]" },
  { action: "gotoPrev", code: "BracketLeft", shift: true, label: "Shift+[" },
  { action: "gotoNext", code: "BracketRight", shift: true, label: "Shift+]" },
  { action: "selectAtPlayhead", code: "Backslash", label: "\\" },
  { action: "clearIn", code: "BracketLeft", alt: true, label: "Alt+[" },
  { action: "clearOut", code: "BracketRight", alt: true, label: "Alt+]" },
  { action: "deleteAll", code: "KeyX", ctrl: true, shift: true, label: "Ctrl+Shift+X" },
  { action: "deleteSelected", code: "Delete", label: "Delete" },
]

const PREMIERE: readonly RangeBinding[] = [
  { action: "markIn", code: "KeyI", label: "I" },
  { action: "markOut", code: "KeyO", label: "O" },
  { action: "gotoPrev", code: "KeyI", shift: true, label: "Shift+I" },
  { action: "gotoNext", code: "KeyO", shift: true, label: "Shift+O" },
  { action: "selectAtPlayhead", code: "KeyX", label: "X" },
  { action: "clearIn", code: "KeyI", ctrl: true, shift: true, label: "Ctrl+Shift+I" },
  { action: "clearOut", code: "KeyO", ctrl: true, shift: true, label: "Ctrl+Shift+O" },
  { action: "deleteAll", code: "KeyX", ctrl: true, shift: true, label: "Ctrl+Shift+X" },
  { action: "deleteSelected", code: "Delete", label: "Delete" },
]

export const RANGE_KEY_PRESETS: Record<PlayRangeKeys, readonly RangeBinding[]> = { brackets: BRACKETS, premiere: PREMIERE }

type KeyLike = Pick<KeyboardEvent, "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey">

/** 프리셋 키 입력 → 구간 동작. 수정자는 정확히 일치해야 하고(Meta 는 항상 제외) 문자 키는 `code` 로 판정한다 */
export function playRangeKeyAction(e: KeyLike, preset: PlayRangeKeys): RangeKeyAction | null {
  if (e.metaKey) return null
  const hit = RANGE_KEY_PRESETS[preset].find(
    (b) => b.code === e.code && !!b.ctrl === e.ctrlKey && !!b.alt === e.altKey && !!b.shift === e.shiftKey
  )
  return hit ? { type: hit.action } : null
}

/** 동작의 화면 표기 (없으면 null) */
export function rangeKeyLabel(preset: PlayRangeKeys, action: RangeKeyActionType): string | null {
  return RANGE_KEY_PRESETS[preset].find((b) => b.action === action)?.label ?? null
}

/** 숨기기 단축키 문자열(`Ctrl+Alt+Shift+키`, 키는 KeyboardEvent.key)의 키 부분 → 물리 키 후보 — 한 글자 / 괄호·역슬래시(Shift 로 바뀐 문자 포함) / Delete */
function codesOfKeyToken(key: string): string[] {
  if (/^[a-zA-Z]$/.test(key)) return [`Key${key.toUpperCase()}`]
  if (key === "[" || key === "{") return ["BracketLeft"]
  if (key === "]" || key === "}") return ["BracketRight"]
  if (key === "\\" || key === "|") return ["Backslash"]
  return key === "Delete" ? ["Delete"] : []
}

/**
 * 설정 `hideShortcut` 문자열이 프리셋의 구간 키와 겹치면 그 키 표기를, 아니면 null.
 * 겹치면 숨기기 단축키가 먼저 처리되어(캡처 단계 stopImmediatePropagation) 해당 구간 키가 동작하지 않는다.
 */
export function presetConflict(preset: PlayRangeKeys, shortcut: string | null): string | null {
  if (shortcut === null) return null
  const parts = shortcut.split("+")
  const key = parts[parts.length - 1] === "" ? "+" : parts[parts.length - 1]
  const mods = new Set(parts.slice(0, -1))
  const codes = codesOfKeyToken(key)
  const hit = RANGE_KEY_PRESETS[preset].find(
    (b) => codes.includes(b.code) && !!b.ctrl === mods.has("Ctrl") && !!b.alt === mods.has("Alt") && !!b.shift === mods.has("Shift")
  )
  return hit?.label ?? null
}
