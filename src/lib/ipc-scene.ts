import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import {
  SCENE_UPDATED_EVENT,
  type CutSource,
  type SceneBackfillResult,
  type SceneCuts,
  type SceneDetectOptions,
  type SceneRequestResult,
  type SceneUpdatedPayload,
} from "./ipc-contract-scene"

export { SCENE_UPDATED_EVENT }

// 장면전환점 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-scene.ts`.

/** 자산의 장면전환점 — 추출한 적 없으면 `state: "none"`. 자산 없음 → not_found */
export function sceneCutsGet(assetId: number): Promise<SceneCuts> {
  return invoke<SceneCuts>("scene_cuts_get", { assetId })
}

/** 현재 위치를 사용자 컷으로 추가 — ms 는 정수여야 한다 (호출부가 반올림해서 보낸다). 서버가 정규화한 전체 결과를 돌려준다 */
export function sceneCutAdd(assetId: number, ms: number): Promise<SceneCuts> {
  return invoke<SceneCuts>("scene_cut_add", { assetId, ms })
}

/** 가장 가까운 컷 1개 제거 — 자동 컷은 숨김 목록으로 옮겨 재추출에도 되살아나지 않는다 */
export function sceneCutRemove(assetId: number, ms: number, source: CutSource): Promise<SceneCuts> {
  return invoke<SceneCuts>("scene_cut_remove", { assetId, ms, source })
}

/** 자동 컷 모두 삭제 (숨김 목록으로 이동) */
export function sceneCutsClearAuto(assetId: number): Promise<SceneCuts> {
  return invoke<SceneCuts>("scene_cuts_clear_auto", { assetId })
}

/** 장면 검출 등록 — 영상만 대기열에 올린다 (사용자 요청은 백필보다 먼저 처리) */
export function sceneDetectRequest(assetIds: number[], options: SceneDetectOptions): Promise<SceneRequestResult> {
  return invoke<SceneRequestResult>("scene_detect_request", { assetIds, options })
}

/** 아직 한 번도 추출하지 않은 활성 영상 전부를 낮은 우선순위로 등록 */
export function sceneDetectBackfill(): Promise<SceneBackfillResult> {
  return invoke<SceneBackfillResult>("scene_detect_backfill")
}

/** 백필 대상 영상 수 */
export function sceneBackfillCount(): Promise<number> {
  return invoke<number>("scene_backfill_count")
}

/** `scene://updated` 구독 — 추출 결과가 커밋된 자산. 반환 함수로 해제한다 */
export function onSceneUpdated(handler: (assetId: number) => void): Promise<UnlistenFn> {
  return listen<SceneUpdatedPayload>(SCENE_UPDATED_EVENT, (e) => handler(e.payload.assetId))
}
