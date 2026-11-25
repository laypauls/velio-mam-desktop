import { MAX_KEYWORD_CHARS, type MetadataValue, type MetadataValueType } from "@/lib/ipc-contract-metadata"

/** 입력 글 → 값 변환 결과. `value: null` = 값 없음(빈 입력). */
export type ParseResult = { ok: true; value: MetadataValue | null } | { ok: false; error: "keywordTooLong" | "numberInvalid" | "dateInvalid" }

const NUMBER_RE = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/

/** 글 입력으로 편집하는 타입 (그 외는 Select·체크 목록) */
export function isTextual(type: MetadataValueType): boolean {
  return type === "keyword" || type === "number" || type === "date"
}

function validDate(y: number, m: number, d: number): boolean {
  if (y < 1 || m < 1 || m > 12 || d < 1) return false
  return d <= new Date(Date.UTC(y, m, 0)).getUTCDate()
}

/** 값 → 입력 칸에 보일 글 (date 는 `YYYY-MM-DD`) */
export function valueToText(value: MetadataValue | null): string {
  if (value === null) return ""
  if (value.kind === "keyword") return value.value
  if (value.kind === "number") return String(value.value)
  if (value.kind === "date") {
    const s = String(value.value).padStart(8, "0")
    return `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}`
  }
  return ""
}

/** 입력 글 → 값. keyword 는 앞뒤 공백 제거 + NFC 후 200자 이하, number 는 유한값만, date 는 유효한 달력 날짜(YYYYMMDD 정수). */
export function parseText(type: "keyword" | "number" | "date", raw: string): ParseResult {
  const text = raw.trim()
  if (text === "") return { ok: true, value: null }
  if (type === "keyword") {
    const value = text.normalize("NFC")
    if ([...value].length > MAX_KEYWORD_CHARS) return { ok: false, error: "keywordTooLong" }
    return { ok: true, value: { kind: "keyword", value } }
  }
  if (type === "number") {
    const n = Number(text)
    if (!NUMBER_RE.test(text) || !Number.isFinite(n)) return { ok: false, error: "numberInvalid" }
    return { ok: true, value: { kind: "number", value: n === 0 ? 0 : n } }
  }
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text)
  if (!m || !validDate(Number(m[1]), Number(m[2]), Number(m[3]))) return { ok: false, error: "dateInvalid" }
  return { ok: true, value: { kind: "date", value: Number(m[1] + m[2] + m[3]) } }
}

/** 두 값이 같은지 (저장 생략·변경 없음 판정) */
export function sameValue(a: MetadataValue | null, b: MetadataValue | null): boolean {
  return JSON.stringify(a) === JSON.stringify(b)
}
