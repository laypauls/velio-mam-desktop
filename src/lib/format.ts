/** 숫자·날짜·크기 표기 — 현재 표시 언어의 Intl (내장 언어 en = en-US·ko = ko-KR). */
import { currentLocale } from "@/i18n/applyLanguage"

interface Formatters {
  number: Intl.NumberFormat
  size: Intl.NumberFormat
  date: Intl.DateTimeFormat
  dateTime: Intl.DateTimeFormat
  month: Intl.DateTimeFormat
  time: Intl.DateTimeFormat
  monthDayTime: Intl.DateTimeFormat
}

const cache = new Map<string, Formatters>()

function build(locale: string): Formatters {
  return {
    number: new Intl.NumberFormat(locale),
    size: new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }),
    date: new Intl.DateTimeFormat(locale, { year: "numeric", month: "2-digit", day: "2-digit" }),
    dateTime: new Intl.DateTimeFormat(locale, {
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
    }),
    month: new Intl.DateTimeFormat(locale, { year: "numeric", month: "long" }),
    time: new Intl.DateTimeFormat(locale, { hour: "numeric", minute: "2-digit" }),
    monthDayTime: new Intl.DateTimeFormat(locale, { month: "numeric", day: "numeric", hour: "numeric", minute: "2-digit" }),
  }
}

/** 현재 언어의 포매터 묶음 — 언어별로 캐시한다. 지원하지 않는 코드는 en-US 로 대체 */
function fmt(): Formatters {
  const locale = currentLocale()
  let f = cache.get(locale)
  if (!f) {
    try {
      f = build(locale)
    } catch {
      f = cache.get("en-US") ?? build("en-US")
    }
    cache.set(locale, f)
  }
  return f
}

export function formatNumber(n: number): string {
  return fmt().number.format(n)
}

export function formatDate(ms: number | null): string {
  return ms === null ? "—" : fmt().date.format(ms)
}

/** 카드처럼 좁은 곳용 날짜 — `2026.11.07` (`formatDate` 와 같은 로케일·시간대, 구분만 간결) */
export function formatDateCompact(ms: number): string {
  const p = Object.fromEntries(fmt().date.formatToParts(ms).map((x) => [x.type, x.value]))
  return `${p.year}.${p.month}.${p.day}`
}

export function formatDateTime(ms: number | null): string {
  return ms === null ? "—" : fmt().dateTime.format(ms)
}

export function formatMonth(ms: number): string {
  return fmt().month.format(ms)
}

/** `YYYY-MM` 그룹 키 (로컬 시간대). */
export function monthKey(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}`
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${fmt().number.format(bytes)} B`
  const units = ["KB", "MB", "GB", "TB"]
  let v = bytes / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${fmt().size.format(v)} ${units[i]}`
}

/** 로컬 시각 기준 `YYYY-MM-DD` (사용량 조회 기간 인자). */
export function dayKey(ms: number): string {
  const d = new Date(ms)
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`
}

/** 밀리초 → `m:ss` / `h:mm:ss`. null 이면 em dash. */
export function formatDuration(ms: number | null): string {
  if (ms === null) return "—"
  const total = Math.max(0, Math.round(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const ss = String(total % 60).padStart(2, "0")
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`
}

/** 시각만 (예: `15:40`·`오후 3:40`) — 현재 언어 표기 */
export function formatTime(ms: number): string {
  return fmt().time.format(ms)
}

/** 월·일 + 시각 (연도 없음) — 표 행처럼 좁은 곳의 먼 날짜용 */
export function formatMonthDayTime(ms: number): string {
  return fmt().monthDayTime.format(ms)
}
