import type { PlayRange } from "@/lib/ipc-contract"

/**
 * 이어보기 위치를 "구간만 재생" 에 맞춘다 (D5). 구간은 정렬·병합된 값이다.
 * 구간 안(시작 포함·끝 제외)이면 그대로, 밖이면 그 뒤 첫 구간의 시작, 뒤에 구간이 없으면 첫 구간의 시작.
 * `ranges` 가 없거나 비어 있으면(전체 재생) 위치를 그대로 돌려준다.
 */
export function fitResumeToRanges(positionMs: number, ranges: readonly PlayRange[] | null | undefined): number {
  if (!ranges || ranges.length === 0) return positionMs
  for (const r of ranges) {
    if (positionMs >= r.startMs && positionMs < r.endMs) return positionMs
    if (positionMs < r.startMs) return r.startMs
  }
  return ranges[0].startMs
}

/** "처음부터" 의 목표 — 구간만 재생 중이면 첫 구간의 시작, 아니면 0 */
export function resumeRestartTarget(ranges: readonly PlayRange[] | null | undefined): number {
  return ranges && ranges.length > 0 ? ranges[0].startMs : 0
}
