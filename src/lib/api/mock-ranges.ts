import type { MamApi, PlayRange } from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import { cmdError } from "./mock-search"

type RangesApi = Pick<MamApi, "playRangesGet" | "playRangesSet" | "collectionPlayRangeSummary">

const MAX_RANGES = 1000
const MOCK_VIDEO_FPS = 30

/**
 * 서버(`normalize_ranges`)와 같은 규칙: 정렬·병합(맞닿음 포함)·길이 clamp·최소 길이 검사.
 * 사용자가 보낸 구간 자체가 최소 길이보다 짧으면 저장 전체를 거부하지만(invalid_input),
 * 길이 clamp 때문에 `최소 길이` 미만이 된 구간은 거부하지 않고 버린다.
 */
export function normalizeMockRanges(ranges: readonly PlayRange[], durationMs: number | null, fps: number | null): PlayRange[] {
  if (ranges.length > MAX_RANGES) throw cmdError("invalid_input", "ranges")
  for (const r of ranges) if (r.startMs < 0 || r.endMs <= r.startMs) throw cmdError("invalid_input", "ranges")
  const minLen = fps !== null && Number.isFinite(fps) && fps > 0 ? Math.max(1, Math.round(1000 / fps)) : 100
  if (ranges.some((r) => r.endMs - r.startMs < minLen)) throw cmdError("invalid_input", "ranges")
  const clipped = ranges
    .map((r) => ({ startMs: r.startMs, endMs: durationMs !== null ? Math.min(r.endMs, durationMs) : r.endMs }))
    .filter((r) => durationMs === null || r.startMs < durationMs)
    .sort((a, b) => a.startMs - b.startMs)
  const merged: PlayRange[] = []
  for (const r of clipped) {
    const last = merged[merged.length - 1]
    if (last && r.startMs <= last.endMs) last.endMs = Math.max(last.endMs, r.endMs)
    else merged.push({ ...r })
  }
  return merged.filter((r) => r.endMs - r.startMs >= minLen)
}

/** 재생 구간 mock — 메모리에만 저장한다. 정수가 아닌 ms 는 실제 Tauri 처럼 command 진입 전 문자열 오류로 거부한다. */
export function createMockRanges({ db, rec, wait }: Pick<MockCtx, "db" | "rec" | "wait">): RangesApi {
  const store = new Map<number, PlayRange[]>()
  return {
    playRangesGet: async (assetId) => {
      await wait()
      rec(assetId)
      return (store.get(assetId) ?? []).map((r) => ({ ...r }))
    },
    playRangesSet: async (assetId, ranges) => {
      await wait()
      if (ranges.some((r) => !Number.isInteger(r.startMs) || !Number.isInteger(r.endMs))) {
        return Promise.reject("invalid type: floating point, expected i64")
      }
      const r = rec(assetId)
      if (r.type === "image") throw cmdError("invalid_input", "mediaType")
      const next = normalizeMockRanges(ranges, r.dur, r.type === "video" ? MOCK_VIDEO_FPS : null)
      if (next.length === 0) store.delete(assetId)
      else store.set(assetId, next)
      return next.map((x) => ({ ...x }))
    },
    collectionPlayRangeSummary: async (collectionId) => {
      await wait()
      const c = db().collections.get(collectionId)
      if (!c) throw cmdError("not_found", "collection not found")
      const out: { assetId: number; rangeCount: number; totalMs: number }[] = []
      for (const [assetId, list] of store) {
        if (!c.members.has(assetId)) continue
        out.push({ assetId, rangeCount: list.length, totalMs: list.reduce((s, x) => s + (x.endMs - x.startMs), 0) })
      }
      return out
    },
  }
}
