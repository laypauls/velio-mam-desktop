import { useEffect } from "react"
import { usePlayQueue } from "@/hooks/usePlayQueue"
import { usePlaybackStore, useViewStore } from "@/stores"

/**
 * 연속 재생과 콘텐츠 보기 영역의 연결 (보기 영역이 열려 있는지와 무관하게 항상 마운트).
 * - 대기열을 최신으로 유지한다 (`usePlayQueue`)
 * - 보기 영역이 닫히면 재생을 멈춘다
 * - 재생 밖에서 다른 자산을 직접 미리보기하면: 대기열 항목이면 그 항목으로 이동, 아니면 재생을 멈추고 일반 미리보기
 *   (재생 스토어 자신의 `openPreview` 는 현재 항목과 같으므로 무시된다)
 */
export function usePlaybackGuards() {
  const collectionId = usePlaybackStore((s) => s.collectionId)
  usePlayQueue(collectionId)
  useEffect(
    () =>
      useViewStore.subscribe((s, prev) => {
        if (s.previewId === prev.previewId) return
        const p = usePlaybackStore.getState()
        if (p.status === "idle") return
        if (s.previewId === null) p.stop()
        else if (s.previewId === p.currentId) return
        else if (p.queue.some((q) => q.assetId === s.previewId)) p.jumpTo(s.previewId)
        else p.stop()
      }),
    []
  )
}
