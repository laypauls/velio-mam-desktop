import { useRef, type KeyboardEvent, type PointerEvent, type RefObject } from "react"
import { useTranslation } from "react-i18next"
import { useViewStore, VIEWER_MIN_WIDTH } from "@/stores/view"

import { measureViewerMaxWidth } from "./viewerFit"

const KEY_STEP = 24

interface Props {
  /** 폭을 조절할 보기 영역 */
  paneRef: RefObject<HTMLElement | null>
  /** 보기 영역이 놓인 쪽 — "right" 면 분할선이 보기 영역 왼쪽 경계(← 가 넓어짐), "left" 면 오른쪽 경계(→ 가 넓어짐) */
  side: "left" | "right"
}

/** 갤러리와 보기 영역 사이의 세로 분할선 — 포인터 드래그와 좌우 화살표로 보기 영역 폭을 조절한다. */
export function ViewerResizeHandle({ paneRef, side }: Props) {
  const { t } = useTranslation()
  const width = useViewStore((s) => s.viewerWidth)
  const setWidth = useViewStore((s) => s.setViewerWidth)
  const dragging = useRef(false)

  const maxWidth = (): number => measureViewerMaxWidth(paneRef.current, side)
  const apply = (next: number) => setWidth(Math.min(Math.max(next, VIEWER_MIN_WIDTH), maxWidth()))

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    dragging.current = true
    e.currentTarget.setPointerCapture?.(e.pointerId)
  }
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const pane = paneRef.current
    if (!dragging.current || !pane) return
    const rect = pane.getBoundingClientRect()
    apply(side === "right" ? rect.right - e.clientX : e.clientX - rect.left)
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return
    e.preventDefault()
    // 분할선이 보기 영역 왼쪽이면 ← 가, 오른쪽이면 → 가 분할선을 바깥으로 밀어 보기 영역을 넓힌다
    const widens = e.key === (side === "right" ? "ArrowLeft" : "ArrowRight")
    apply(width + (widens ? KEY_STEP : -KEY_STEP))
  }

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("viewer.resize")}
      aria-valuenow={width}
      aria-valuemin={VIEWER_MIN_WIDTH}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={() => (dragging.current = false)}
      onPointerCancel={() => (dragging.current = false)}
      onKeyDown={onKeyDown}
      className="w-1.5 shrink-0 cursor-col-resize touch-none bg-border/0 transition-colors outline-none hover:bg-border focus-visible:bg-ring/50"
    />
  )
}
