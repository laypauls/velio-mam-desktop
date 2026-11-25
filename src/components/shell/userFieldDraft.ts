import type { MetadataField, UserFieldFilter } from "@/lib/ipc-contract-metadata"
import { dateToInt, intToDate, parseNumber, type Presence } from "@/lib/userFieldFilters"

/** 필드 하나의 입력 중 상태 — 유효할 때만 조건(`UserFieldFilter`)으로 바뀌어 필터에 실린다 */
export interface FieldDraft {
  presence: Presence
  keywords: string[]
  keywordText: string
  /** number: 숫자 텍스트, date: `YYYY-MM-DD` */
  min: string
  max: string
  bool: "any" | "true" | "false"
  optionIds: number[]
  all: boolean
}

export function initialDraft(cond: UserFieldFilter | undefined): FieldDraft {
  const d: FieldDraft = { presence: "any", keywords: [], keywordText: "", min: "", max: "", bool: "any", optionIds: [], all: false }
  if (!cond) return d
  switch (cond.kind) {
    case "has":
    case "missing":
      return { ...d, presence: cond.kind }
    case "keywordAnyOf":
      return { ...d, keywords: cond.values }
    case "numberRange":
      return { ...d, min: cond.min === null ? "" : String(cond.min), max: cond.max === null ? "" : String(cond.max) }
    case "dateRange":
      return { ...d, min: intToDate(cond.min), max: intToDate(cond.max) }
    case "bool":
      return { ...d, bool: cond.value ? "true" : "false" }
    case "enumAnyOf":
      return { ...d, optionIds: cond.optionIds }
    case "multiEnum":
      return { ...d, optionIds: cond.optionIds, all: cond.all }
  }
}

/** 입력 상태 → 조건. `invalid` 면 보내지 않는다(min > max). `cond: null` = 이 필드는 필터 없음 */
export function buildCondition(field: MetadataField, d: FieldDraft): { cond: UserFieldFilter | null; invalid: boolean } {
  const fieldId = field.fieldId
  if (d.presence !== "any") return { cond: { kind: d.presence, fieldId }, invalid: false }
  switch (field.valueType) {
    case "keyword":
      return { cond: d.keywords.length > 0 ? { kind: "keywordAnyOf", fieldId, values: d.keywords } : null, invalid: false }
    case "number":
    case "date": {
      const parse = field.valueType === "number" ? parseNumber : dateToInt
      const min = parse(d.min)
      const max = parse(d.max)
      if (min === null && max === null) return { cond: null, invalid: false }
      if (min !== null && max !== null && min > max) return { cond: null, invalid: true }
      return { cond: { kind: field.valueType === "number" ? "numberRange" : "dateRange", fieldId, min, max }, invalid: false }
    }
    case "boolean":
      return { cond: d.bool === "any" ? null : { kind: "bool", fieldId, value: d.bool === "true" }, invalid: false }
    case "enum":
    case "multiEnum": {
      // 선택지 표시 순서대로, 현재 정의에 있는 것만
      const ids = field.options.map((o) => o.optionId).filter((id) => d.optionIds.includes(id))
      if (ids.length === 0) return { cond: null, invalid: false }
      return {
        cond: field.valueType === "enum" ? { kind: "enumAnyOf", fieldId, optionIds: ids } : { kind: "multiEnum", fieldId, optionIds: ids, all: d.all },
        invalid: false,
      }
    }
  }
}
