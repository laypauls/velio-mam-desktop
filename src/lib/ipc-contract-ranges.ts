/**
 * 재생 구간(자산 단위) 계약 — `docs/requests/2026-11/20261113-014827-player-input-and-play-ranges.md`.
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 */

/** 재생 구간 단축키 프리셋 — brackets: [ ] \ 등, premiere: I O X 등 */
export type PlayRangeKeys = "brackets" | "premiere"

/** 재생 구간 (정수 ms, 서버가 정렬·병합해 돌려준다) */
export interface PlayRange {
  startMs: number
  endMs: number
}

/** `collection_play_range_summary` 항목 — 구간이 있는 항목만 (희소) */
export interface PlayRangeSummary {
  assetId: number
  rangeCount: number
  totalMs: number
}

export interface RangesApi {
  playRangesGet(assetId: number): Promise<PlayRange[]> // play_ranges_get { assetId } — start 오름차순, 없으면 []
  playRangesSet(assetId: number, ranges: PlayRange[]): Promise<PlayRange[]> // play_ranges_set { assetId, ranges } — 전체 교체, 서버가 정렬·병합한 결과를 돌려준다. ms 는 정수만
  collectionPlayRangeSummary(collectionId: number): Promise<PlayRangeSummary[]> // collection_play_range_summary { collectionId } — 구간이 있는 항목만
}
