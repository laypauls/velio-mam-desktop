import type { MetadataField, MetadataValueType, UserFieldFilter } from "@/lib/ipc-contract-metadata"

/** 필드 하나에 대한 "값 유무" 선택 — any 면 타입별 값 입력이 조건이 된다 */
export type Presence = "any" | "has" | "missing"

/** 값 조건 종류 → 필드 타입. `has`/`missing` 은 모든 타입에 쓸 수 있다(null). */
const KIND_TYPE: Record<UserFieldFilter["kind"], MetadataValueType | null> = {
  keywordAnyOf: "keyword",
  numberRange: "number",
  dateRange: "date",
  bool: "boolean",
  enumAnyOf: "enum",
  multiEnum: "multiEnum",
  has: null,
  missing: null,
}

/**
 * 현재 필드 정의에 없는 조건을 걷어낸다 — 없는(삭제된) 필드, 타입이 다른 필드, 없는 선택지.
 * 선택지가 일부만 없으면 남은 선택지로 조건을 유지하고, 하나도 안 남으면 조건을 뺀다.
 * 바뀐 게 없으면 입력과 같은 배열을 돌려준다(상태 갱신 루프 방지). `dropped` 는 빠졌거나 줄어든 조건 수.
 */
export function pruneUserFields(conds: UserFieldFilter[], fields: MetadataField[]): { kept: UserFieldFilter[]; dropped: number } {
  const byId = new Map(fields.map((f) => [f.fieldId, f]))
  const kept: UserFieldFilter[] = []
  let dropped = 0
  for (const c of conds) {
    const field = byId.get(c.fieldId)
    const type = KIND_TYPE[c.kind]
    if (!field || (type !== null && type !== field.valueType)) {
      dropped++
      continue
    }
    if (c.kind === "enumAnyOf" || c.kind === "multiEnum") {
      const known = new Set(field.options.map((o) => o.optionId))
      const ids = c.optionIds.filter((id) => known.has(id))
      if (ids.length === 0) {
        dropped++
        continue
      }
      if (ids.length !== c.optionIds.length) {
        dropped++
        kept.push({ ...c, optionIds: ids })
        continue
      }
    }
    kept.push(c)
  }
  return dropped === 0 ? { kept: conds, dropped } : { kept, dropped }
}

/** `YYYY-MM-DD`(date 입력 값) → `YYYYMMDD` 정수. 달력에 없는 날짜·형식 오류는 null */
export function dateToInt(text: string): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(text)
  if (!m) return null
  const [y, mo, d] = [Number(m[1]), Number(m[2]), Number(m[3])]
  const probe = new Date(Date.UTC(y, mo - 1, d))
  if (probe.getUTCFullYear() !== y || probe.getUTCMonth() !== mo - 1 || probe.getUTCDate() !== d) return null
  return y * 10_000 + mo * 100 + d
}

/** `YYYYMMDD` 정수 → `YYYY-MM-DD` (null → 빈 문자열) */
export function intToDate(v: number | null): string {
  if (v === null) return ""
  const s = String(v).padStart(8, "0")
  return `${s.slice(0, 4)}-${s.slice(4, 6)}-${s.slice(6, 8)}`
}

/** 숫자 입력 텍스트 → 유한한 숫자 (빈 값·해석 불가 = null) */
export function parseNumber(text: string): number | null {
  if (text.trim() === "") return null
  const n = Number(text)
  return Number.isFinite(n) ? n : null
}

/** keyword 값 정규화 — 앞뒤 공백 제거 + NFC (백엔드와 같은 규칙, 빈 값은 ""). */
export function normalizeKeyword(raw: string): string {
  return raw.trim().normalize("NFC")
}

/** 조건 목록을 필터에 싣는 값 — 비면 키 자체를 빼서(undefined) 기본 필터와 같은 JSON 이 되게 한다 */
export function userFieldsOrUndefined(list: UserFieldFilter[]): UserFieldFilter[] | undefined {
  return list.length > 0 ? list : undefined
}
