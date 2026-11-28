import { useLayoutEffect, type RefObject } from "react"
import { useViewStore } from "@/stores/view"
import { panelFitsWithViewer } from "@/components/viewer/viewerFit"

/**
 * 보기 영역이 열려 있고 본문 행 폭이 모자라면 섹션 패널을 임시로 접는다(저장된 `sidebarOpen` 은 그대로).
 * 사용자가 로고·섹션으로 다시 펼친 상태(`panelForced`)는 보기 영역이 닫히거나 접힘 판정이 바뀔 때까지 존중한다.
 * 상세 패널은 사용자가 연 것이라 자동으로 닫지 않는다 — 폭 계산에 넣을 뿐이다.
 * 분할 보기 영역이 없는 경우(닫힘·필름스트립)에는 아무것도 접지 않는다.
 */
export function usePanelAutoCollapse(rowRef: RefObject<HTMLElement | null>): void {
  const viewerOpen = useViewStore((s) => s.previewId !== null && s.viewMode !== "filmstrip")
  const detailOpen = useViewStore((s) => s.detailOpen)
  const setCollapsed = useViewStore((s) => s.setPanelAutoCollapsed)
  const clearForced = useViewStore((s) => s.clearPanelForced)

  useLayoutEffect(() => {
    const row = rowRef.current
    if (!viewerOpen || !row) {
      setCollapsed(false)
      clearForced()
      return
    }
    const measure = () => setCollapsed(!panelFitsWithViewer(row.getBoundingClientRect().width, detailOpen))
    measure()
    window.addEventListener("resize", measure)
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure)
    observer?.observe(row)
    return () => {
      window.removeEventListener("resize", measure)
      observer?.disconnect()
    }
  }, [rowRef, viewerOpen, detailOpen, setCollapsed, clearForced])
}
