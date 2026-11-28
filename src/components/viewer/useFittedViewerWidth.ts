import { useCallback, useLayoutEffect, useState, type RefObject } from "react"
import { selectPanelVisible, useViewStore } from "@/stores/view"
import { fitViewerWidth, measureViewerMaxWidth } from "./viewerFit"

/**
 * 보기 영역의 화면 표시 폭 — 저장된 폭(`viewerWidth`)을 현재 창에 들어가도록 줄인 값.
 * 창 크기·섹션 패널/상세 열림이 바뀔 때마다 다시 계산하며 저장 값은 건드리지 않아, 창을 다시 키우면 사용자가 정한 폭으로 돌아온다.
 * 왼쪽·오른쪽 배치 모두 본문 행 안에서 형제 `main`(갤러리)과 폭을 나누므로 같은 계산이다. 레일 폭은 고정이라 `main` 관찰로 충분하다.
 */
export function useFittedViewerWidth(paneRef: RefObject<HTMLElement | null>): number {
  const width = useViewStore((s) => s.viewerWidth)
  const panelVisible = useViewStore(selectPanelVisible)
  const detailOpen = useViewStore((s) => s.detailOpen)
  const [max, setMax] = useState(Number.POSITIVE_INFINITY)

  const measure = useCallback(() => setMax(measureViewerMaxWidth(paneRef.current)), [paneRef])

  useLayoutEffect(() => {
    measure()
    const parent = paneRef.current?.parentElement
    window.addEventListener("resize", measure)
    let observer: ResizeObserver | undefined
    if (parent && typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(measure)
      // 본문 행과 갤러리 폭 변화(패널·상세 열림 포함)를 감지한다 — 보기 영역 자신은 관찰하지 않는다(되먹임 방지)
      observer.observe(parent)
      const main = parent.querySelector(":scope > main")
      if (main) observer.observe(main)
    }
    return () => {
      window.removeEventListener("resize", measure)
      observer?.disconnect()
    }
  }, [measure, paneRef, panelVisible, detailOpen])

  return fitViewerWidth(width, max)
}
