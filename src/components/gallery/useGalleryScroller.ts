import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react"

/** 휠 한 줄(deltaMode=1)의 픽셀 환산 */
export const WHEEL_LINE_PX = 40
/** 이 속도(px/ms)를 넘는 이동은 "빠른 스크롤" — 블록 요청을 멈출 때까지 미룬다 */
export const FAST_PX_PER_MS = 4

export interface GalleryScroller {
  /** 현재 스크롤 오프셋 (콘텐츠 y, 0 ~ maxOffset) */
  offset: number
  maxOffset: number
  /** `jump` = 레일·키보드·앵커 같은 목적지 이동 (빠른 스크롤로 보지 않는다) */
  scrollTo: (y: number, opts?: { jump?: boolean }) => void
  scrollBy: (dy: number, opts?: { jump?: boolean }) => void
  /** 마지막 이동이 빠른 스크롤이었는가 — `requestRange` 의 `fast` */
  isFast: () => boolean
  /** 이동이 있을 때마다 호출 (레일 자동 표시용). 해제 함수를 돌려준다 */
  subscribe: (cb: () => void) => () => void
  /** 휠 한 번 처리 — 뷰포트와 목차 레일이 같은 함수를 쓴다(줄/쪽 단위 환산·빠른 스크롤 판정 동일). 처리했으면 preventDefault 한다 */
  handleWheel: (e: WheelEvent) => void
}

interface Args {
  /** 휠을 받는 요소 (콜백 ref 로 얻은 state) */
  el: HTMLElement | null
  contentH: number
  viewportH: number
}

/**
 * 오프셋 직접 관리 스크롤 (결정 A). 콘텐츠 높이가 WebView2 레이아웃 한계(≈33.5M px)를 넘을 수 있어 네이티브 스크롤을 쓰지 않고,
 * 휠을 가로채 오프셋만 갱신한다. DOM 에는 보이는 행만 그린다.
 */
export function useGalleryScroller({ el, contentH, viewportH }: Args): GalleryScroller {
  const maxOffset = Math.max(0, contentH - viewportH)
  const [raw, setRaw] = useState(0)
  const offset = Math.min(maxOffset, Math.max(0, raw))
  const live = useRef({ offset, maxOffset, viewportH })
  live.current = { offset, maxOffset, viewportH }
  const last = useRef({ t: 0, y: 0 })
  const fast = useRef(false)
  const listeners = useRef(new Set<() => void>())

  // 콘텐츠가 줄어 오프셋이 잘렸다면 상태에도 반영한다 (다시 커져도 옛 위치로 튀지 않게)
  useLayoutEffect(() => {
    if (raw !== offset) setRaw(offset)
  }, [raw, offset])

  const scrollTo = useCallback((y: number, opts?: { jump?: boolean }) => {
    const ny = Math.min(live.current.maxOffset, Math.max(0, y))
    if (ny === live.current.offset) return
    const t = performance.now()
    fast.current = !opts?.jump && Math.abs(ny - last.current.y) / Math.max(1, t - last.current.t) > FAST_PX_PER_MS
    last.current = { t, y: ny }
    live.current.offset = ny // 같은 틱의 연속 휠 이벤트가 누적되게
    setRaw(ny)
    listeners.current.forEach((cb) => cb())
  }, [])
  const scrollBy = useCallback((dy: number, opts?: { jump?: boolean }) => scrollTo(live.current.offset + dy, opts), [scrollTo])
  const isFast = useCallback(() => fast.current, [])
  const subscribe = useCallback((cb: () => void) => {
    listeners.current.add(cb)
    return () => void listeners.current.delete(cb)
  }, [])

  const handleWheel = useCallback(
    (e: WheelEvent) => {
      // 확대(Ctrl+휠)와 가로 스크롤(목록 최소 폭 아래)은 브라우저에 맡긴다
      if (e.ctrlKey || e.deltaY === 0 || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return
      e.preventDefault()
      const d = e.deltaMode === 1 ? e.deltaY * WHEEL_LINE_PX : e.deltaMode === 2 ? e.deltaY * live.current.viewportH : e.deltaY
      scrollBy(d)
    },
    [scrollBy]
  )
  useEffect(() => {
    if (!el) return
    el.addEventListener("wheel", handleWheel, { passive: false })
    return () => el.removeEventListener("wheel", handleWheel)
  }, [el, handleWheel])

  return { offset, maxOffset, scrollTo, scrollBy, isFast, subscribe, handleWheel }
}
