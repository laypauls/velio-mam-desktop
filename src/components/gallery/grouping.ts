import { formatMonth } from "@/lib/format"
import type { SortField } from "@/lib/ipc-contract"

/** D7: 촬영일/수정일 정렬일 때만 월 그룹 머리글. */
export function groupField(field: SortField): "capturedAt" | "modifiedAt" | null {
  return field === "capturedAt" || field === "modifiedAt" ? field : null
}

/** `search_index` 월 구간 키(`YYYY-MM`)의 머리글 문구. `none`(날짜 없음)·형식이 다른 키는 null — 호출부가 번역 키로 대체한다 */
export function monthSectionLabel(key: string): string | null {
  const m = /^(\d{4})-(\d{2})$/.exec(key)
  return m ? formatMonth(new Date(Number(m[1]), Number(m[2]) - 1, 15).getTime()) : null
}
