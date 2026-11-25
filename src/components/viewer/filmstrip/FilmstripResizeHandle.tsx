import { useRef, type KeyboardEvent, type PointerEvent } from "react"
import { useTranslation } from "react-i18next"
import { FILMSTRIP_MAX_HEIGHT, FILMSTRIP_MIN_HEIGHT, useViewStore } from "@/stores/view"

const KEY_STEP = 8

/** 보기 무대와 스트립 사이의 가로 분할선 — 위로 끌면(또는 ↑) 스트립이 커진다. 저장은 `layout.filmstripHeight`. */
export function FilmstripResizeHandle() {
  const { t } = useTranslation()
  const height = useViewStore((s) => s.filmstripHeight)
  const setHeight = useViewStore((s) => s.setFilmstripHeight)
  const drag = useRef<{ y: number; h: number } | null>(null)

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    drag.current = { y: e.clientY, h: useViewStore.getState().filmstripHeight }
    e.currentTarget.setPointerCapture?.(e.pointerId)
  }
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const from = drag.current
    if (from) setHeight(from.h + (from.y - e.clientY))
  }
  const end = () => {
    drag.current = null
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowUp" && e.key !== "ArrowDown") return
    e.preventDefault()
    setHeight(height + (e.key === "ArrowUp" ? KEY_STEP : -KEY_STEP))
  }
  return (
    <div
      role="separator"
      aria-orientation="horizontal"
      aria-label={t("viewer.filmstrip.resize")}
      aria-valuenow={height}
      aria-valuemin={FILMSTRIP_MIN_HEIGHT}
      aria-valuemax={FILMSTRIP_MAX_HEIGHT}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
      onKeyDown={onKeyDown}
      className="group grid h-2 shrink-0 cursor-row-resize touch-none place-items-center border-t border-border outline-none focus-visible:bg-ring/30"
    >
      <span aria-hidden="true" className="h-0.5 w-9 rounded-full bg-border group-hover:bg-muted-foreground" />
    </div>
  )
}
