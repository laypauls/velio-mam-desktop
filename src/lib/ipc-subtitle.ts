import { invoke } from "@tauri-apps/api/core"
import type { SubtitleCue, SubtitleTrack } from "./ipc-contract-subtitle"

// 동영상 외부 자막 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-subtitle.ts`.

/** 영상 옆 자막 트랙 (정렬됨, 없으면 []). 저장 장치 오프라인 → asset_offline */
export function subtitleTracks(assetId: number): Promise<SubtitleTrack[]> {
  return invoke<SubtitleTrack[]>("subtitle_tracks", { assetId })
}

/** 트랙의 큐 목록 (`<video>` 경로) */
export function subtitleCues(assetId: number, trackId: string): Promise<SubtitleCue[]> {
  return invoke<SubtitleCue[]>("subtitle_cues", { assetId, trackId })
}

/** 내장 플레이어의 자막 선택 — `null` = 끔. 다른 자산의 요청은 무시된다 */
export function playerSetSubtitle(assetId: number, trackId: string | null): Promise<void> {
  return invoke<void>("player_set_subtitle", { assetId, trackId })
}
