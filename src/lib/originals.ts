import type { TFunction } from "i18next"
import { formatNumber } from "@/lib/format"
import { ORIGINALS_FAILURE_CODES, type OriginalsResult } from "@/lib/ipc-contract"

const KNOWN: ReadonlySet<string> = new Set(ORIGINALS_FAILURE_CODES)

/** 자산별 실패 사유를 코드별로 묶어 "파일 없음 2건, 오프라인 1건" 처럼 많은 순으로 요약한다. */
export function summarizeFailures(t: TFunction, failed: OriginalsResult["failed"]): string {
  const counts = new Map<string, number>()
  for (const f of failed) {
    const code = KNOWN.has(f.message) ? f.message : "other"
    counts.set(code, (counts.get(code) ?? 0) + 1)
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .map(([code, n]) => t("originals.reasonCount", { reason: t(`originals.reason.${code}`), count: formatNumber(n) }))
    .join(", ")
}
