import { useEffect } from "react"
import { usePlaybackStore } from "@/stores"

/**
 * 재생 화면이 "재생할 수 없음" 상태가 되면 연속 재생에 알린다 — 대기열이 그 항목을 건너뛰고 다음으로 넘어간다.
 * 연속 재생 중인 현재 항목이 아니면 스토어가 무시하므로 일반 미리보기에는 영향이 없다.
 */
export function usePlaybackFailure(assetId: number, failed: boolean): void {
  useEffect(() => {
    if (failed) usePlaybackStore.getState().reportFailed(assetId)
  }, [assetId, failed])
}
