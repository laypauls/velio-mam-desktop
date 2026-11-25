import type { SceneCut } from "@/lib/ipc-contract"

/** 우클릭·hover 가 컷을 잡는 거리(px) */
export const HIT_RADIUS_PX = 4
/** 삼각형 SVG 의 세로 좌표계 높이(px) — 사용자 컷 삼각형 높이와 같다 */
export const LAYER_HEIGHT = 9
/** 삼각형을 합치는 칸 폭(px) — 삼각형은 폭이 10~12px 라 선분(1px)보다 칸을 넓혀 겹침과 path 길이를 줄인다 */
export const CUT_BUCKET_PX = 4

/** 역삼각형 크기(폭×높이) — 자동은 작게, 사용자 컷은 크게 (색 외에 크기로도 구분) */
export const CUT_SIZE = { auto: { w: 10, h: 7 }, user: { w: 12, h: 9 } } as const

export interface CutPaths {
  auto: string
  user: string
}

/** 꼭짓점이 아래를 향한 삼각형 path — `x` 는 위쪽 변의 중심 */
export function trianglePath(x: number, source: "auto" | "user", scale = 1): string {
  const { w, h } = CUT_SIZE[source]
  const hw = (w * scale) / 2
  return `M${x - hw} 0h${hw * 2}l${-hw} ${h * scale}z`
}

/** 소스별 역삼각형 path — 트랙 폭을 4px 칸으로 나눠 같은 칸에 든 컷은 삼각형 하나로 합친다 (컷 수와 무관하게 path 는 소스당 1개, 삼각형은 폭/4 + 1 이하) */
export function buildCutPaths(cuts: readonly SceneCut[], durationMs: number, trackPx: number): CutPaths {
  if (durationMs <= 0 || trackPx <= 0) return { auto: "", user: "" }
  const seen = { auto: new Set<number>(), user: new Set<number>() }
  const parts = { auto: [] as string[], user: [] as string[] }
  for (const c of cuts) {
    const x = Math.round((Math.min(Math.max(c.ms, 0), durationMs) / durationMs) * trackPx)
    const bucket = Math.floor(x / CUT_BUCKET_PX)
    if (seen[c.source].has(bucket)) continue
    seen[c.source].add(bucket)
    parts[c.source].push(trianglePath(x, c.source))
  }
  return { auto: parts.auto.join(""), user: parts.user.join("") }
}

/**
 * 트랙 시작(썸 반지름만큼 안쪽)에서 `xPx` 떨어진 위치 둘레 `radiusPx` 안의 가장 가까운 컷 — 없으면 null.
 * `cuts` 는 ms 오름차순(서버가 병합해 준다). 이분 탐색이라 컷이 수만 개여도 포인터 이동마다 부담이 없다.
 */
export function nearestCut(cuts: readonly SceneCut[], durationMs: number, trackPx: number, xPx: number, radiusPx = HIT_RADIUS_PX): SceneCut | null {
  if (cuts.length === 0 || durationMs <= 0 || trackPx <= 0) return null
  const ms = (xPx / trackPx) * durationMs
  let lo = 0
  let hi = cuts.length
  while (lo < hi) {
    const mid = (lo + hi) >> 1
    if (cuts[mid].ms < ms) lo = mid + 1
    else hi = mid
  }
  let best: SceneCut | null = null
  for (const c of [cuts[lo - 1], cuts[lo]]) {
    if (!c) continue
    const dx = Math.abs((c.ms / durationMs) * trackPx - xPx)
    if (dx <= radiusPx && (best === null || dx < Math.abs((best.ms / durationMs) * trackPx - xPx))) best = c
  }
  return best
}

/** 툴팁·접근성 표기 — `mm:ss.mmm` (1시간 이상이면 `h:mm:ss.mmm`) */
export function formatSceneTime(ms: number): string {
  const total = Math.max(0, Math.round(ms))
  const h = Math.floor(total / 3_600_000)
  const m = Math.floor((total % 3_600_000) / 60_000)
  const s = Math.floor((total % 60_000) / 1000)
  const mmm = String(total % 1000).padStart(3, "0")
  const ss = String(s).padStart(2, "0")
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}.${mmm}` : `${String(m).padStart(2, "0")}:${ss}.${mmm}`
}
