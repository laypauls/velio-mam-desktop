/**
 * 동영상 장면전환점 계약 — `docs/requests/2026-11/20261113-212724-architect-scene-cuts-design.md` §5.
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 */

/** 검출 민감도 — 설정 `sceneDetectSensitivity` 와 "다시 추출" 요청에 쓴다 */
export type SceneSensitivity = "low" | "medium" | "high"

/** 추출 상태 — `none` 은 행 없음(추출 요청된 적 없음), `pending` 은 대기 또는 실행 중 */
export type SceneState = "none" | "pending" | "done" | "failed" | "unsupported"

export type CutSource = "auto" | "user"

/** 실패 사유 코드 (경로·stderr 는 로그에만 남고 내려오지 않는다) */
export type SceneFailure = "tool_not_found" | "timeout" | "tool_failed" | "output_too_large" | "io" | "other"

export interface SceneCut {
  /** 정수 ms */
  ms: number
  source: CutSource
}

/** `scene_cuts_get` 및 편집 command 의 반환 — 자동·사용자 컷이 ms 오름차순으로 병합돼 있다 */
export interface SceneCuts {
  assetId: number
  state: SceneState
  /** 마지막 추출에 쓴 민감도 — 사용자 컷만 있는 행(추출 한 적 없음)은 null */
  sensitivity: SceneSensitivity | null
  error: SceneFailure | null
  cuts: SceneCut[]
  /** 사용자가 지워서 숨겨 둔 자동 컷 수 */
  suppressedCount: number
  updatedAt: number | null
}

/** `scene_detect_request` 옵션 — 모두 생략하면 새로 추출(이미 done 이면 건너뜀) */
export interface SceneDetectOptions {
  /** done 이어도 다시 추출 */
  redetect?: boolean
  /** 지운 자동 컷(숨김 목록)을 되살린다 */
  restoreSuppressed?: boolean
  /** 이 요청에만 쓸 민감도 (생략 = 설정값) */
  sensitivity?: SceneSensitivity | null
}

export interface SceneRequestResult {
  queued: number
  skipped: number
}

export interface SceneBackfillResult {
  queued: number
}

/** `scene://updated` 페이로드 — 추출 실행기가 결과를 커밋한 뒤에만 온다 */
export interface SceneUpdatedPayload {
  assetId: number
}

export const SCENE_UPDATED_EVENT = "scene://updated"

/** 컷 위치 허용 오차 계산에 쓰는 fps 가 없을 때의 기본 값 (서버와 같은 표) */
export const SCENE_TOLERANCE_FALLBACK_MS = 100
/** 한 자산의 사용자 컷 상한 (서버 `MAX_USER_CUTS`) */
export const MAX_USER_SCENE_CUTS = 5000

export interface SceneApi {
  sceneCutsGet(assetId: number): Promise<SceneCuts> // scene_cuts_get { assetId } — 행 없음 = state "none". 자산 없음 → not_found
  sceneCutAdd(assetId: number, ms: number): Promise<SceneCuts> // scene_cut_add { assetId, ms } — 정수 ms. 허용 오차 안에 이미 있으면 no-op. 영상 아님 → invalid_input("mediaType"), 범위 밖 → ("ms"), 5,001번째 → ("cuts")
  sceneCutRemove(assetId: number, ms: number, source: CutSource): Promise<SceneCuts> // scene_cut_remove { assetId, ms, source } — 허용 오차 안 최근접 1개. auto 는 숨김 목록으로, 없으면 no-op
  sceneCutsClearAuto(assetId: number): Promise<SceneCuts> // scene_cuts_clear_auto { assetId } — 자동 컷 전부 숨김 목록으로
  sceneDetectRequest(assetIds: number[], options: SceneDetectOptions): Promise<SceneRequestResult> // scene_detect_request { assetIds, options } — 활성 영상만 등록(사용자 요청 = 높은 우선순위). 0개·10,000 초과 → invalid_input("assetIds")
  sceneDetectBackfill(): Promise<SceneBackfillResult> // scene_detect_backfill — 행 없는 활성 영상 전부를 낮은 우선순위로 등록
  sceneBackfillCount(): Promise<number> // scene_backfill_count — 백필 대상 영상 수
  onSceneUpdated(handler: (assetId: number) => void): Promise<() => void> // event scene://updated
}
