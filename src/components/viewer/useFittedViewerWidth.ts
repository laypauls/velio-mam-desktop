import { useCallback, useLayoutEffect, useState, type RefObject } from "react"
import { useViewStore } from "@/stores"
import { fitViewerWidth, measureViewerMaxWidth, type ViewerSide } from "./viewerFit"

/**
 * 보기 영역의 화면 표시 폭 — 저장된 폭(`viewerWidth`)을 현재 창에 들어가도록 줄인 값.
 * 창 크기·사이드바/상세 열림이 바뀔 때마다 다시 계산하며 저장 값은 건드리지 않아, 창을 다시 키우면 사용자가 정한 폭으로 돌아온다.
 */
export function useFittedViewerWidth(paneRef: RefObject<HTMLElement | null>, side: ViewerSide): number {
  const width = useViewStore((s) => s.viewerWidth)
  const sidebarOpen = useViewStore((s) => s.sidebarOpen)
  const [max, setMax] = useState(Number.POSITIVE_INFINITY)

  const measure = useCallback(() => setMax(measureViewerMaxWidth(paneRef.current, side)), [paneRef, side])

  useLayoutEffect(() => {
    measure()
    const pane = paneRef.current
    const parent = pane?.parentElement
    window.addEventListener("resize", measure)
    let observer: ResizeObserver | undefined
    if (parent && typeof ResizeObserver !== "undefined") {
      observer = new ResizeObserver(measure)
      // 부모·형제 열 + (왼쪽 배치의) 사이드바·상세 폭 변화를 감지한다 — 보기 영역 자신은 관찰하지 않는다(되먹임 방지)
      observer.observe(parent)
      const sibling = parent.querySelector(side === "left" ? ":scope > [data-viewer-sibling]" : ":scope > main")
      if (sibling) observer.observe(sibling)
      for (const el of sibling?.querySelector("main")?.parentElement?.children ?? []) if (el !== pane) observer.observe(el)
    }
    return () => {
      window.removeEventListener("resize", measure)
      observer?.disconnect()
    }
  }, [measure, paneRef, side, sidebarOpen])

  return fitViewerWidth(width, max)
}
