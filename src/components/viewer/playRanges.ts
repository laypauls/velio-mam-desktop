import type { PlayRange } from "@/lib/ipc-contract"

/** 구간 경계 판정 결과 — Rust `mam_player::ranges::RangeAction` 과 같은 규칙 */
export type RangeAction = { kind: "none" } | { kind: "seek"; toMs: number } | { kind: "end"; atMs: number }

const NONE: RangeAction = { kind: "none" }

/**
 * 현재 위치 `tMs` 에서 구간만 재생이 해야 할 일. `ranges` 는 정렬·병합된 구간(서버 정규화 결과).
 * 구간 앞/사이 → 다음 구간 시작으로 seek, 구간 안 → 없음, 마지막 구간 끝/뒤 → 끝(마지막 구간 끝 위치).
 * `lookaheadMs` 는 프레임/폴링 주기만큼 미리 끝으로 간주해 넘침을 줄인다. 공유 테스트 벡터: 설계 §4.3 (Rust `t_player_ranges_01` 과 동일).
 * `startToleranceMs` (기본 0 = 공유 벡터와 동일) — 구간 시작보다 이만큼 이른 위치도 그 구간 안으로 본다.
 * `<video>` 가 시작 시각으로 seek 해도 실제로 표시되는 프레임은 그 직전 프레임 시각(예: 1444 → 1440)이거나 부동소수 오차(1443.9999)라
 * 허용치가 없으면 같은 시작으로 무한 seek 한다.
 */
export function nextRangeAction(tMs: number, ranges: readonly PlayRange[], lookaheadMs: number, startToleranceMs = 0): RangeAction {
  if (ranges.length === 0) return NONE
  for (const r of ranges) {
    if (tMs + startToleranceMs < r.startMs) return { kind: "seek", toMs: r.startMs }
    if (tMs + lookaheadMs < r.endMs) return NONE
  }
  return { kind: "end", atMs: ranges[ranges.length - 1].endMs }
}

/** `<audio>` 의 lookahead — rAF 주기(~16ms)의 절반 */
export const AUDIO_LOOKAHEAD_MS = 8
/** fps 를 모를 때 `<video>` 의 lookahead */
const VIDEO_LOOKAHEAD_FALLBACK_MS = 17

/** `<video>` 프레임 콜백 lookahead = round(500 / fps) (반 프레임) */
export function videoLookaheadMs(fps: number | null): number {
  return fps !== null && Number.isFinite(fps) && fps > 0 ? Math.round(500 / fps) : VIDEO_LOOKAHEAD_FALLBACK_MS
}

/** 구간 시작 허용치 — 1프레임(fps 를 모르면 40ms) */
export function startToleranceMs(fps: number | null): number {
  return fps !== null && Number.isFinite(fps) && fps > 0 ? Math.max(1, Math.round(1000 / fps)) : 40
}

/** 최소 구간 길이(ms) — 서버 규칙과 같다: fps 가 유효하면 1프레임, 아니면 100ms */
export function minRangeMs(fps: number | null): number {
  return fps !== null && Number.isFinite(fps) && fps > 0 ? Math.max(1, Math.round(1000 / fps)) : 100
}

/** 서버가 받는 정수 ms 로 맞춘다 (소수는 command 진입 전에 문자열 오류로 거부된다) */
export function roundRanges(ranges: readonly PlayRange[]): PlayRange[] {
  return ranges.map((r) => ({ startMs: Math.round(r.startMs), endMs: Math.round(r.endMs) }))
}

/** 정렬 + 겹침/맞닿음 병합 — 서버 정규화 결과를 기다리는 동안 보여 줄 낙관적 값 */
export function mergeRanges(ranges: readonly PlayRange[]): PlayRange[] {
  const sorted = [...ranges].sort((a, b) => a.startMs - b.startMs)
  const out: PlayRange[] = []
  for (const r of sorted) {
    const last = out[out.length - 1]
    if (last && r.startMs <= last.endMs) out[out.length - 1] = { startMs: last.startMs, endMs: Math.max(last.endMs, r.endMs) }
    else out.push({ ...r })
  }
  return out
}

export function totalRangesMs(ranges: readonly PlayRange[]): number {
  return ranges.reduce((sum, r) => sum + (r.endMs - r.startMs), 0)
}

/** `ms` 를 포함하는(양끝 포함) 구간의 인덱스 — 없으면 -1 */
export function rangeIndexAt(ranges: readonly PlayRange[], ms: number): number {
  return ranges.findIndex((r) => ms >= r.startMs && ms <= r.endMs)
}

/** 위치 판정 허용 오차(ms) — libVLC 시간 갱신 지연으로 방금 이동한 경계가 조금 어긋나 보이는 것을 흡수한다 */
export const RANGE_NAV_TOLERANCE_MS = 250

/** 현재 위치보다 앞에 있는 마지막 구간의 시작 — "이전 구간 시작으로 이동" */
export function prevRangeStart(ranges: readonly PlayRange[], ms: number): number | null {
  for (let i = ranges.length - 1; i >= 0; i--) if (ranges[i].startMs < ms - RANGE_NAV_TOLERANCE_MS) return ranges[i].startMs
  return null
}

/** 현재 위치보다 뒤에 끝나는 첫 구간의 끝 — "다음 구간 끝으로 이동" */
export function nextRangeEnd(ranges: readonly PlayRange[], ms: number): number | null {
  for (const r of ranges) if (r.endMs > ms + RANGE_NAV_TOLERANCE_MS) return r.endMs
  return null
}

/** 구간 시각 표기 `m:ss.d` (시간이 있으면 `h:mm:ss.d`) — 구간은 초 미만 경계가 의미 있어 formatDuration 보다 촘촘하게 보인다 */
export function formatRangeTime(ms: number): string {
  const clamped = Math.max(0, Math.round(ms))
  const tenths = Math.floor((clamped % 1000) / 100)
  const total = Math.floor(clamped / 1000)
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const ss = String(total % 60).padStart(2, "0")
  return `${h > 0 ? `${h}:${String(m).padStart(2, "0")}` : m}:${ss}.${tenths}`
}
