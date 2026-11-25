import type { TFunction } from "i18next"
import {
  KNOWN_TRIGGER_KINDS,
  type KnownTriggerKind,
  type RescanPolicy,
  type StoredRescanPolicy,
  type Weekday,
} from "./ipc-contract-watch"

/** 서버 검증과 같은 범위 (Rust `INTERVAL_MIN_HOURS`/`INTERVAL_MAX_HOURS` — 하한은 잠정) */
export const INTERVAL_HOURS = { min: 1, max: 720 } as const

export const WEEKDAYS: readonly Weekday[] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
const TIME_RE = /^([01]\d|2[0-3]):[0-5]\d$/

const isRecord = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v)
const isTime = (v: unknown): v is string => typeof v === "string" && TIME_RE.test(v)

/**
 * Rust `parse_policy` 와 같은 필드 경로를 돌려준다 (null = 유효). 공유 fixture
 * `crates/mam-core/tests/fixtures/rescan_policy_cases.json` 으로 동등성을 검증한다 (T-ui-pol-02).
 */
export function validateRescanPolicy(p: unknown): string | null {
  if (!isRecord(p)) return "rescanPolicy"
  if (Object.keys(p).some((k) => k !== "trigger" && k !== "window" && k !== "onStart")) return "rescanPolicy"
  const trigger = p.trigger
  if (!isRecord(trigger)) return "rescanPolicy.trigger"
  const kind = trigger.kind
  if (typeof kind !== "string" || !(KNOWN_TRIGGER_KINDS as readonly string[]).includes(kind)) return "rescanPolicy.trigger.kind"
  const allowed = kind === "interval" ? ["kind", "hours"] : kind === "schedule" ? ["kind", "days", "at"] : ["kind"]
  if (Object.keys(trigger).some((k) => !allowed.includes(k))) return "rescanPolicy.trigger"
  if (kind === "interval") {
    const h = trigger.hours
    if (typeof h !== "number" || !Number.isInteger(h) || h < INTERVAL_HOURS.min || h > INTERVAL_HOURS.max) return "rescanPolicy.trigger.hours"
  }
  if (kind === "schedule") {
    const d = trigger.days
    if (!Array.isArray(d) || d.some((x) => !(WEEKDAYS as readonly unknown[]).includes(x))) return "rescanPolicy.trigger.days"
    if (!isTime(trigger.at)) return "rescanPolicy.trigger.at"
  }
  const w = p.window
  if (w !== undefined && w !== null) {
    if (!isRecord(w) || Object.keys(w).some((k) => k !== "start" && k !== "end")) return "rescanPolicy.window"
    if (!isTime(w.start)) return "rescanPolicy.window.start"
    if (!isTime(w.end)) return "rescanPolicy.window.end"
    if (w.start === w.end) return "rescanPolicy.window"
  }
  if (p.onStart !== undefined && typeof p.onStart !== "boolean") return "rescanPolicy.onStart"
  return null
}

/** 서버와 같은 정규화 — 요일은 월→일 순·중복 제거, 7일 모두 = 빈 배열(매일), window·onStart 기본값 채움. 유효한 입력만 넘길 것 */
export function normalizeRescanPolicy(p: RescanPolicy): RescanPolicy {
  const { trigger } = p
  const window = p.window ?? null
  if (trigger.kind !== "schedule") return { trigger, window, onStart: p.onStart === true }
  const days = WEEKDAYS.filter((d) => trigger.days.includes(d))
  return { trigger: { kind: "schedule", days: days.length === 7 ? [] : days, at: trigger.at }, window, onStart: p.onStart === true }
}

/** 이 버전 UI 가 편집할 수 있는 저장값인가 (알 수 없는 kind·깨진 값이면 false) */
export function isKnownPolicy(p: StoredRescanPolicy | null): p is StoredRescanPolicy & RescanPolicy {
  return p !== null && validateRescanPolicy(p) === null
}

// ─── 편집 초안 ──────────────────────────────────────────────────────────────

export const POLICY_DEFAULT = "__default__"
export const POLICY_UNKNOWN = "__unknown__"
export type DraftKind = KnownTriggerKind | typeof POLICY_DEFAULT | typeof POLICY_UNKNOWN

export interface PolicyDraft {
  kind: DraftKind
  /** 입력 중 문자열 (비어 있거나 숫자가 아닐 수 있다) */
  hours: string
  days: Weekday[]
  at: string
  windowEnabled: boolean
  windowStart: string
  windowEnd: string
  onStart: boolean
}

const BLANK: Omit<PolicyDraft, "kind" | "hours"> = {
  days: [],
  at: "03:00",
  windowEnabled: false,
  windowStart: "22:00",
  windowEnd: "06:00",
  onStart: false,
}

export function draftFromPolicy(stored: StoredRescanPolicy | null, defaultHours: number): PolicyDraft {
  const base = { ...BLANK, hours: String(defaultHours) }
  if (stored === null) return { ...base, kind: POLICY_DEFAULT }
  if (!isKnownPolicy(stored)) return { ...base, kind: POLICY_UNKNOWN }
  const { trigger, window, onStart } = stored
  return {
    ...base,
    kind: trigger.kind,
    ...(trigger.kind === "interval" ? { hours: String(trigger.hours) } : {}),
    ...(trigger.kind === "schedule" ? { days: [...trigger.days], at: trigger.at } : {}),
    ...(window ? { windowEnabled: true, windowStart: window.start, windowEnd: window.end } : {}),
    onStart: onStart === true,
  }
}

export interface DraftErrors {
  hours?: "range"
  at?: "time"
  windowStart?: "time"
  windowEnd?: "time"
  window?: "same"
}

/** 시간대 조건은 자동 트리거(간격·요일/시각)에만 의미가 있다 */
export function windowVisible(d: PolicyDraft): boolean {
  return d.kind === "interval" || d.kind === "schedule"
}

/** 입력 칸 단위 오류. 화면에 보이지 않는 칸(다른 종류의 옵션)은 검사하지 않는다 */
export function validateDraft(d: PolicyDraft): DraftErrors {
  const e: DraftErrors = {}
  if (d.kind === "interval") {
    const n = Number(d.hours)
    if (d.hours.trim() === "" || !Number.isInteger(n) || n < INTERVAL_HOURS.min || n > INTERVAL_HOURS.max) e.hours = "range"
  }
  if (d.kind === "schedule" && !isTime(d.at)) e.at = "time"
  if (windowVisible(d) && d.windowEnabled) {
    if (!isTime(d.windowStart)) e.windowStart = "time"
    if (!isTime(d.windowEnd)) e.windowEnd = "time"
    if (!e.windowStart && !e.windowEnd && d.windowStart === d.windowEnd) e.window = "same"
  }
  return e
}

/** 저장 결과가 같은가 — 현재 종류에 보이지 않는 칸의 값은 비교하지 않는다 */
export const sameDraft = (a: PolicyDraft, b: PolicyDraft) => {
  if (a.kind !== b.kind) return false
  if (a.kind === POLICY_DEFAULT || a.kind === POLICY_UNKNOWN) return true
  if (a.onStart !== b.onStart) return false
  if (a.kind === "interval" && a.hours !== b.hours) return false
  if (a.kind === "schedule" && (a.at !== b.at || a.days.length !== b.days.length || !a.days.every((d) => b.days.includes(d)))) return false
  if (windowVisible(a) && (a.windowEnabled !== b.windowEnabled || (a.windowEnabled && (a.windowStart !== b.windowStart || a.windowEnd !== b.windowEnd)))) return false
  return true
}

/**
 * 초안 → 저장할 정책. `"unchanged"` = 건드리지 않았거나 알 수 없는 정책이라 만들 수 없음 (호출하지 않는다),
 * `null` = 전역 기본으로 되돌림. 오류가 있는 초안은 넘기지 말 것 (`validateDraft` 가 비어 있을 때만).
 */
export function policyFromDraft(draft: PolicyDraft, initial: PolicyDraft): RescanPolicy | null | "unchanged" {
  if (sameDraft(draft, initial) || draft.kind === POLICY_UNKNOWN) return "unchanged"
  if (draft.kind === POLICY_DEFAULT) return null
  const window = windowVisible(draft) && draft.windowEnabled ? { start: draft.windowStart, end: draft.windowEnd } : null
  const trigger: RescanPolicy["trigger"] =
    draft.kind === "interval"
      ? { kind: "interval", hours: Number(draft.hours) }
      : draft.kind === "schedule"
        ? { kind: "schedule", days: WEEKDAYS.filter((d) => draft.days.includes(d)), at: draft.at }
        : { kind: "manual" }
  return normalizeRescanPolicy({ trigger, window, onStart: draft.onStart })
}

// ─── 표시 ───────────────────────────────────────────────────────────────────

const sameDays = (days: readonly Weekday[], want: readonly Weekday[]) =>
  days.length === want.length && want.every((d) => days.includes(d))

/** 폴더 행·요약용 한 줄 설명 (예: "6시간마다 (기본)", "평일 03:00, 시작 시 1회") */
export function describePolicy(t: TFunction, stored: StoredRescanPolicy | null, defaultHours: number): string {
  const k = "settings.folders.detection.summary"
  if (stored === null) return t(`${k}.default`, { hours: defaultHours })
  if (!isKnownPolicy(stored)) return t(`${k}.unknown`)
  const { trigger, window, onStart } = stored
  let main: string
  if (trigger.kind === "interval") main = t(`${k}.interval`, { hours: trigger.hours })
  else if (trigger.kind === "manual") main = t(`${k}.manual`)
  else if (trigger.days.length === 0 || trigger.days.length === 7) main = t(`${k}.daily`, { at: trigger.at })
  else if (sameDays(trigger.days, ["mon", "tue", "wed", "thu", "fri"])) main = t(`${k}.weekdays`, { at: trigger.at })
  else if (sameDays(trigger.days, ["sat", "sun"])) main = t(`${k}.weekends`, { at: trigger.at })
  else {
    const days = WEEKDAYS.filter((d) => trigger.days.includes(d))
      .map((d) => t(`watch.weekday.${d}`))
      .join(" ")
    main = t(`${k}.days`, { days, at: trigger.at })
  }
  const parts = [main]
  if (window) parts.push(t(`${k}.window`, { start: window.start, end: window.end }))
  if (onStart) parts.push(t(`${k}.onStart`))
  return parts.join(", ")
}

/** 저장값의 트리거 종류 (알 수 없으면 null) */
export function storedKind(stored: StoredRescanPolicy | null): KnownTriggerKind | null {
  return isKnownPolicy(stored) ? stored.trigger.kind : null
}
