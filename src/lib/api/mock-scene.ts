import { MAX_USER_SCENE_CUTS, SCENE_TOLERANCE_FALLBACK_MS } from "@/lib/ipc-contract-scene"
import type { CutSource, MamApi, SceneCuts, SceneFailure, SceneSensitivity, SceneState } from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import { cmdError } from "./mock-search"

export const SCENE_SENSITIVITIES: readonly SceneSensitivity[] = ["low", "medium", "high"]

type SceneApiSlice = Pick<
  MamApi,
  "sceneCutsGet" | "sceneCutAdd" | "sceneCutRemove" | "sceneCutsClearAuto" | "sceneDetectRequest" | "sceneDetectBackfill" | "sceneBackfillCount" | "onSceneUpdated"
>

const MOCK_VIDEO_FPS = 30
const MAX_REQUEST_IDS = 10_000
/** mock 자동 검출이 영상을 나누는 구간 수 */
const MOCK_AUTO_PARTS = 8

interface Row {
  state: Exclude<SceneState, "none">
  sensitivity: SceneSensitivity | null
  requested: SceneSensitivity | null
  auto: number[]
  user: number[]
  suppressed: number[]
  error: SceneFailure | null
  updatedAt: number
}

/** 서버(`tolerance_ms`)와 같은 표 — 유효 fps 면 한 프레임(≥1ms), 아니면 100ms */
export function mockToleranceMs(fps: number | null): number {
  return fps !== null && Number.isFinite(fps) && fps > 0 ? Math.max(1, Math.round(1000 / fps)) : SCENE_TOLERANCE_FALLBACK_MS
}

const insertSorted = (list: number[], ms: number) => {
  list.push(ms)
  list.sort((a, b) => a - b)
}
/** `target` 의 ±tol 안에서 가장 가까운 항목의 인덱스 (없으면 -1) */
const nearestIndex = (list: readonly number[], target: number, tol: number): number => {
  let best = -1
  for (let i = 0; i < list.length; i++) {
    const d = Math.abs(list[i] - target)
    if (d <= tol && (best < 0 || d < Math.abs(list[best] - target))) best = i
  }
  return best
}

/**
 * 장면전환점 mock — 메모리에만 저장하고 서버 규칙을 재현한다: 허용 오차(±1프레임, 30fps → 33ms), 자동 컷 제거 = 숨김 목록,
 * 사용자 컷은 재추출에도 유지, 영상만(mediaType), 행 없음 = state "none", 행 없는 자산에 add → state "done"(sensitivity null).
 * 정수가 아닌 ms 는 실제 Tauri 처럼 command 진입 전 문자열 오류로 거부한다.
 */
export function createMockScene({ db, rec, wait, now, settings, opts }: MockCtx): SceneApiSlice {
  const rows = new Map<number, Row>()
  const handlers = new Set<(assetId: number) => void>()
  const schedule = opts.scene?.schedule ?? ((fn: () => void, ms: number) => void setTimeout(fn, ms))
  const stepMs = opts.scene?.stepMs ?? 800

  const view = (assetId: number): SceneCuts => {
    const row = rows.get(assetId)
    if (!row) return { assetId, state: "none", sensitivity: null, error: null, cuts: [], suppressedCount: 0, updatedAt: null }
    const cuts = [
      ...row.auto.map((ms) => ({ ms, source: "auto" as const })),
      ...row.user.map((ms) => ({ ms, source: "user" as const })),
    ].sort((a, b) => a.ms - b.ms)
    return { assetId, state: row.state, sensitivity: row.sensitivity, error: row.error, cuts, suppressedCount: row.suppressed.length, updatedAt: row.updatedAt }
  }
  const video = (assetId: number) => {
    const r = rec(assetId)
    if (r.type !== "video") throw cmdError("invalid_input", "mediaType")
    return r
  }
  const touch = (row: Row) => void (row.updatedAt = now())
  const blank = (state: Row["state"]): Row => ({ state, sensitivity: null, requested: null, auto: [], user: [], suppressed: [], error: null, updatedAt: now() })

  /** 실행기 완료 흉내 — 숨김·사용자 컷 ±tol 을 제외하고 자동 컷을 교체한 뒤 이벤트를 보낸다 */
  const complete = (assetId: number) => {
    const row = rows.get(assetId)
    const r = db().recs[assetId - 1]
    if (!row || row.state !== "pending" || !r) return
    const tol = mockToleranceMs(MOCK_VIDEO_FPS)
    const dur = r.dur ?? 0
    const raw = dur > 0 ? Array.from({ length: MOCK_AUTO_PARTS - 1 }, (_, i) => Math.round(((i + 1) * dur) / MOCK_AUTO_PARTS)) : []
    row.auto = raw.filter((ms) => nearestIndex(row.suppressed, ms, tol) < 0 && nearestIndex(row.user, ms, tol) < 0)
    row.sensitivity = row.requested ?? settings.sceneDetectSensitivity
    row.requested = null
    row.error = null
    row.state = "done"
    touch(row)
    for (const h of [...handlers]) h(assetId)
  }

  const editable = (assetId: number, ms?: number) => {
    if (ms !== undefined && !Number.isInteger(ms)) throw "invalid type: floating point, expected i64"
    return video(assetId)
  }
  const isBackfillTarget = (i: number) => {
    const r = db().recs[i]
    return r.type === "video" && r.status === "active" && !rows.has(i + 1)
  }

  return {
    sceneCutsGet: async (assetId) => {
      await wait()
      rec(assetId)
      return view(assetId)
    },
    sceneCutAdd: async (assetId, ms) => {
      await wait()
      const r = editable(assetId, ms)
      if (ms < 0 || (r.dur !== null && ms > r.dur)) throw cmdError("invalid_input", "ms")
      const row = rows.get(assetId)
      const tol = mockToleranceMs(MOCK_VIDEO_FPS)
      if (row && (nearestIndex(row.auto, ms, tol) >= 0 || nearestIndex(row.user, ms, tol) >= 0)) return view(assetId)
      if (row && row.user.length >= MAX_USER_SCENE_CUTS) throw cmdError("invalid_input", "cuts")
      const next = row ?? blank("done")
      insertSorted(next.user, ms)
      touch(next)
      rows.set(assetId, next)
      return view(assetId)
    },
    sceneCutRemove: async (assetId, ms, source: CutSource) => {
      await wait()
      editable(assetId, ms)
      const row = rows.get(assetId)
      if (!row) return view(assetId)
      const list = source === "auto" ? row.auto : row.user
      const at = nearestIndex(list, ms, mockToleranceMs(MOCK_VIDEO_FPS))
      if (at < 0) return view(assetId)
      const [removed] = list.splice(at, 1)
      if (source === "auto") insertSorted(row.suppressed, removed)
      touch(row)
      return view(assetId)
    },
    sceneCutsClearAuto: async (assetId) => {
      await wait()
      video(assetId)
      const row = rows.get(assetId)
      if (row && row.auto.length > 0) {
        for (const ms of row.auto) insertSorted(row.suppressed, ms)
        row.auto = []
        touch(row)
      }
      return view(assetId)
    },
    sceneDetectRequest: async (assetIds, options) => {
      await wait()
      if (assetIds.length === 0 || assetIds.length > MAX_REQUEST_IDS) throw cmdError("invalid_input", "assetIds")
      if (options.sensitivity != null && !SCENE_SENSITIVITIES.includes(options.sensitivity)) throw cmdError("invalid_input", "sensitivity")
      let queued = 0
      let skipped = 0
      for (const id of new Set(assetIds)) {
        const r = db().recs[id - 1]
        if (!r || r.type !== "video" || r.status !== "active") {
          skipped++
          continue
        }
        const row = rows.get(id)
        if (row && row.state === "done" && !options.redetect) {
          skipped++
          continue
        }
        const next = row ?? blank("pending")
        next.state = "pending"
        next.error = null
        next.requested = options.sensitivity ?? null
        if (options.restoreSuppressed) next.suppressed = []
        touch(next)
        rows.set(id, next)
        queued++
        schedule(() => complete(id), stepMs)
      }
      return { queued, skipped }
    },
    sceneDetectBackfill: async () => {
      await wait()
      let queued = 0
      for (let i = 0; i < db().recs.length; i++) {
        if (!isBackfillTarget(i)) continue
        rows.set(i + 1, blank("pending"))
        queued++
      }
      return { queued }
    },
    sceneBackfillCount: async () => {
      await wait()
      let n = 0
      for (let i = 0; i < db().recs.length; i++) if (isBackfillTarget(i)) n++
      return n
    },
    onSceneUpdated: async (handler) => {
      handlers.add(handler)
      return () => void handlers.delete(handler)
    },
  }
}
