import type { Locale } from "react-day-picker"
import { de } from "react-day-picker/locale/de"
import { enUS } from "react-day-picker/locale/en-US"
import { es } from "react-day-picker/locale/es"
import { fr } from "react-day-picker/locale/fr"
import { ja } from "react-day-picker/locale/ja"
import { ko } from "react-day-picker/locale/ko"

const LOCALES: Record<string, Locale> = { ko, en: enUS, ja, es, fr, de }

/** i18n 언어 코드(`ko`, `en-GB` 등) → 달력 로케일. 모르는 코드는 en-US. */
export function calendarLocale(language: string | undefined): Locale {
  const primary = (language ?? "").toLowerCase().split(/[-_]/)[0]
  return LOCALES[primary] ?? enUS
}

/** 로컬 날짜 → `YYYY-MM-DD` */
export function formatDateText(d: Date): string {
  return `${String(d.getFullYear()).padStart(4, "0")}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`
}

/** 실제로 존재하는 `YYYY-MM-DD` 만 Date(로컬 자정)로, 그 외(2026-02-30 포함)는 undefined */
export function parseDateText(text: string): Date | undefined {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text)
  if (!m) return undefined
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])]
  const date = new Date(y, mo - 1, d)
  return date.getFullYear() === y && date.getMonth() === mo - 1 && date.getDate() === d ? date : undefined
}
