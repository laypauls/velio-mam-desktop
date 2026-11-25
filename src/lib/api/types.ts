import type { MamApi, ThumbSize } from "@/lib/ipc-contract"
import type { AnalysisChange } from "@/lib/ipc-contract-ai"

/** 커스텀 프로토콜 URL 빌더 — 썸네일(`mamthumb`)·원본 재생(`mammedia`). */
export interface MediaUrls {
  thumbUrl(assetId: number, size: ThumbSize): string
  mediaUrl(assetId: number): string
}

/** `ai://analysis` 구독 — 분석 상태가 바뀐 자산(최대 100개/이벤트)의 전체 단계 상태를 받는다. */
export interface AnalysisEvents {
  onAnalysisChanged(handler: (changes: AnalysisChange[]) => void): Promise<() => void>
}

/** 화면이 쓰는 API 묶음 (코어 command + URL 헬퍼 + 분석 이벤트). */
export type UiApi = MamApi & MediaUrls & AnalysisEvents
