import { useEffect, useRef } from "react"
import { useViewerNavStore, type GalleryStepper } from "@/stores"

/**
 * 갤러리의 "열린 항목 이전/다음" 이동을 보기 영역(플레이어 컨트롤 바)에 내준다.
 * 이동할 곳이 없으면(결과 1개, 열린 항목이 이 목록에 없음) 내주지 않고, 갤러리가 사라지면 거둔다.
 * `step` 은 매 렌더 새 클로저라 ref 로 최신 것을 부른다.
 */
export function usePublishViewerNav({ canPrev, canNext, step }: GalleryStepper) {
  const stepRef = useRef(step)
  stepRef.current = step
  useEffect(() => {
    const { setStepper } = useViewerNavStore.getState()
    setStepper(canPrev || canNext ? { canPrev, canNext, step: (dir) => stepRef.current(dir) } : null)
  }, [canPrev, canNext])
  useEffect(() => () => useViewerNavStore.getState().setStepper(null), [])
}
