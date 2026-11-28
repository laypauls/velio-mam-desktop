import type { SubtitleSegment } from "@/lib/ipc-contract-ai"

/** 시작 시각 순으로 정렬한 사본 — 이미 정렬돼 있으면 같은 배열을 그대로 돌려준다 (구간 목록은 보통 정렬돼 있다) */
export function sortSegments(segments: readonly SubtitleSegment[]): readonly SubtitleSegment[] {
  for (let i = 1; i < segments.length; i++) {
    if (segments[i].startMs < segments[i - 1].startMs) return [...segments].sort((a, b) => a.startMs - b.startMs)
  }
  return segments
}

/**
 * 시각(ms)에 표시할 구간 — 시작 포함·끝 제외, 구간 사이 빈 시간은 null.
 * `sorted` 는 `sortSegments` 결과여야 한다 (이진 탐색: 시작 ≤ 시각 인 마지막 구간).
 */
export function findActiveSegment(sorted: readonly SubtitleSegment[], ms: number): SubtitleSegment | null {
  let lo = 0
  let hi = sorted.length - 1
  let found = -1
  while (lo <= hi) {
    const mid = (lo + hi) >> 1
    if (sorted[mid].startMs <= ms) {
      found = mid
      lo = mid + 1
    } else hi = mid - 1
  }
  if (found < 0) return null
  const seg = sorted[found]
  return ms < seg.endMs ? seg : null
}
