import { useCallback, useEffect, useRef, type DragEvent } from "react"
import { useSelectionStore } from "@/stores"
import type { ReorderActions } from "./useReorderActions"

/** 가장자리에서 자동 스크롤을 시작하는 거리(px) */
export const AUTO_SCROLL_EDGE = 48
/** 가장자리에 닿았을 때 프레임당 최대 이동량(px) */
export const AUTO_SCROLL_MAX = 28
/** 이 앱의 순서 재배치 드래그임을 알리는 MIME — 다른 드래그(파일·텍스트)는 받지 않는다 */
export const REORDER_MIME = "application/x-mam-reorder"

interface Args {
  reorder: ReorderActions
  layout: "grid" | "list"
  /** 오프셋 스크롤러의 이동 — 네이티브 스크롤이 없어 직접 움직인다 */
  scrollBy: (dy: number) => void
  /** 자동 스크롤 기준이 되는 뷰포트 요소 */
  viewport: HTMLElement | null
}

export interface ReorderDrag {
  /** 목록 래퍼(컨텍스트 메뉴 트리거와 같은 요소)에 펼친다 — 카드는 `[data-asset-id]` 로 찾는다 */
  dragProps: {
    onDragStart: (e: DragEvent<HTMLElement>) => void
    onDragOver: (e: DragEvent<HTMLElement>) => void
    onDrop: (e: DragEvent<HTMLElement>) => void
    onDragEnd: () => void
    onDragLeave: (e: DragEvent<HTMLElement>) => void
  }
  /** 놓을 위치 표시선 — 래퍼 안에 한 번만 그린다. 위치는 DOM 으로 직접 갱신해 카드를 다시 그리지 않는다 */
  indicatorRef: (el: HTMLDivElement | null) => void
}

function cardOf(e: DragEvent<HTMLElement>): HTMLElement | null {
  return e.target instanceof Element ? e.target.closest<HTMLElement>("[data-asset-id]") : null
}

/**
 * 컬렉션 안 카드 드래그 재배치 (HTML5 DnD, 격자·목록 공통).
 * 드래그 대상 = 선택된 카드면 선택 전체, 아니면 그 카드. 카드 위에 놓으면 그 앞으로, 빈 곳이면 맨 뒤로, 자기 자신이면 변화 없음.
 * 위·아래 가장자리 근처에서는 rAF 루프로 자동 스크롤한다 (드래그 시작~끝에서만 돈다).
 */
export function useReorderDrag({ reorder, layout, scrollBy, viewport }: Args): ReorderDrag {
  const dragged = useRef<readonly number[] | null>(null)
  const indicator = useRef<HTMLDivElement | null>(null)
  const pointerY = useRef<number | null>(null)
  const raf = useRef<number | null>(null)
  const live = useRef({ reorder, scrollBy, viewport })
  live.current = { reorder, scrollBy, viewport }

  const hideIndicator = useCallback(() => {
    if (indicator.current) indicator.current.hidden = true
  }, [])

  const stopScroll = useCallback(() => {
    if (raf.current !== null) cancelAnimationFrame(raf.current)
    raf.current = null
    pointerY.current = null
  }, [])

  const tick = useCallback(() => {
    const { viewport: vp, scrollBy: by } = live.current
    const y = pointerY.current
    if (vp && y !== null) {
      const rect = vp.getBoundingClientRect()
      const fromTop = y - rect.top
      const fromBottom = rect.bottom - y
      if (fromTop < AUTO_SCROLL_EDGE) by(-Math.ceil((AUTO_SCROLL_MAX * (AUTO_SCROLL_EDGE - Math.max(0, fromTop))) / AUTO_SCROLL_EDGE))
      else if (fromBottom < AUTO_SCROLL_EDGE) by(Math.ceil((AUTO_SCROLL_MAX * (AUTO_SCROLL_EDGE - Math.max(0, fromBottom))) / AUTO_SCROLL_EDGE))
    }
    raf.current = requestAnimationFrame(tick)
  }, [])

  const end = useCallback(() => {
    dragged.current = null
    hideIndicator()
    stopScroll()
  }, [hideIndicator, stopScroll])

  // 드래그 도중 언마운트(보기 전환 등)해도 루프가 남지 않게
  useEffect(() => stopScroll, [stopScroll])

  const showIndicator = (card: HTMLElement | null, wrapper: HTMLElement) => {
    const el = indicator.current
    if (!el) return
    const base = wrapper.getBoundingClientRect()
    if (card) {
      const r = card.getBoundingClientRect()
      el.hidden = false
      if (layout === "grid") {
        el.style.left = `${r.left - base.left - 3}px`
        el.style.top = `${r.top - base.top}px`
        el.style.width = "3px"
        el.style.height = `${r.height}px`
      } else {
        el.style.left = "0px"
        el.style.top = `${r.top - base.top - 1}px`
        el.style.width = `${base.width}px`
        el.style.height = "2px"
      }
    } else el.hidden = true
  }

  const onDragStart = (e: DragEvent<HTMLElement>) => {
    const card = cardOf(e)
    const id = card ? Number(card.dataset.assetId) : NaN
    if (!live.current.reorder.enabled || !card || Number.isNaN(id)) return
    const sel = useSelectionStore.getState().ids
    dragged.current = sel.has(id) ? [...sel] : [id]
    e.dataTransfer?.setData(REORDER_MIME, String(dragged.current.length))
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move"
    if (raf.current === null) raf.current = requestAnimationFrame(tick)
  }

  const onDragOver = (e: DragEvent<HTMLElement>) => {
    if (dragged.current === null) return
    e.preventDefault() // 놓을 수 있는 자리임을 알린다
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move"
    pointerY.current = e.clientY
    const card = cardOf(e)
    const id = card ? Number(card.dataset.assetId) : NaN
    // 자기 자신(또는 같이 끌리는 카드) 위에서는 표시선을 그리지 않는다
    showIndicator(card && !dragged.current.includes(id) ? card : null, e.currentTarget)
  }

  const onDrop = (e: DragEvent<HTMLElement>) => {
    const ids = dragged.current
    if (ids === null) return
    e.preventDefault()
    const card = cardOf(e)
    const target = card ? Number(card.dataset.assetId) : null
    end()
    if (target !== null && ids.includes(target)) return
    live.current.reorder.dropBefore(ids, target)
  }

  const onDragLeave = (e: DragEvent<HTMLElement>) => {
    // 래퍼 밖으로 나갔을 때만 (자식 사이 이동은 무시)
    if (!(e.relatedTarget instanceof Node) || !e.currentTarget.contains(e.relatedTarget)) hideIndicator()
  }

  const indicatorRef = useCallback((el: HTMLDivElement | null) => {
    indicator.current = el
  }, [])

  return { dragProps: { onDragStart, onDragOver, onDrop, onDragEnd: end, onDragLeave }, indicatorRef }
}
