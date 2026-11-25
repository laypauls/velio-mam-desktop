import { useLayoutEffect, useState } from "react"

/**
 * 요소 content 폭을 추적한다 (ResizeObserver). 요소는 callback ref 로 state 에 담아 전달한다.
 * 폭 0 측정은 무시하고 마지막 유효 폭을 유지한다 — 요소가 숨겨지면(display:none 등) 0 이 보고되어
 * 가상 그리드가 열 수·행을 다시 계산하고 스크롤이 초기화되는 것을 막는다.
 */
export function useElementWidth(el: HTMLElement | null): number {
  const [width, setWidth] = useState(0)
  useLayoutEffect(() => {
    if (!el) return
    const apply = (w: number) => {
      if (w > 0) setWidth(Math.round(w))
    }
    apply(el.clientWidth)
    if (typeof ResizeObserver === "undefined") return
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width
      if (w !== undefined) apply(w)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [el])
  return width
}

/** 요소 content 폭·높이 — 0 측정은 무시하고 마지막 유효 값을 유지한다 (`useElementWidth` 와 같은 이유) */
export function useElementSize(el: HTMLElement | null): { width: number; height: number } {
  const [size, setSize] = useState({ width: 0, height: 0 })
  useLayoutEffect(() => {
    if (!el) return
    const apply = (w: number, h: number) => {
      const width = Math.round(w)
      const height = Math.round(h)
      setSize((s) => {
        const next = { width: width > 0 ? width : s.width, height: height > 0 ? height : s.height }
        return next.width === s.width && next.height === s.height ? s : next
      })
    }
    apply(el.clientWidth, el.clientHeight)
    if (typeof ResizeObserver === "undefined") return
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect
      if (r) apply(r.width, r.height)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [el])
  return size
}
