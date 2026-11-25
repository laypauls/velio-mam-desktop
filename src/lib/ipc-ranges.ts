import { invoke } from "@tauri-apps/api/core"
import type { PlayRange, PlayRangeSummary } from "./ipc-contract-ranges"

// 재생 구간·플레이어 구간 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-ranges.ts`, `ipc-contract-player.ts`.

/** 자산의 재생 구간 (start 오름차순, 없으면 []). 자산 없음 → not_found */
export function playRangesGet(assetId: number): Promise<PlayRange[]> {
  return invoke<PlayRange[]>("play_ranges_get", { assetId })
}

/**
 * 재생 구간 전체 교체 — 서버가 정렬·병합한 결과를 돌려준다. 이미지·너무 짧은 구간·1000개 초과는 invalid_input.
 * ms 는 정수여야 한다 (소수는 command 진입 전에 문자열 오류로 거부되므로 호출부가 반올림해서 보낸다).
 */
export function playRangesSet(assetId: number, ranges: PlayRange[]): Promise<PlayRange[]> {
  return invoke<PlayRange[]>("play_ranges_set", { assetId, ranges })
}

/** 컬렉션 항목 중 구간이 있는 것만 (희소) */
export function collectionPlayRangeSummary(collectionId: number): Promise<PlayRangeSummary[]> {
  return invoke<PlayRangeSummary[]>("collection_play_range_summary", { collectionId })
}

/** 구간만 재생 — `null`/`[]` = 끔. 열 때마다 구간이 초기화되므로 열기 성공 뒤마다 다시 보낸다. 다른 자산의 요청은 무시된다 */
export function playerSetRanges(assetId: number, ranges: PlayRange[] | null): Promise<void> {
  return invoke<void>("player_set_ranges", { assetId, ranges })
}
