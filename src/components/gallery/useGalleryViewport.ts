import { useEffect, useLayoutEffect, useMemo, useRef } from "react"
import { useGalleryWindow, type GalleryWindow } from "@/hooks/galleryWindow/useGalleryWindow"
import type { GalleryPaging, SearchIndex } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { firstVisibleItem, offsetOfItemTop, offsetToReveal, useGalleryGeometry, visibleItems, type GalleryGeometry } from "./galleryGeometry"
import type { GalleryNav } from "./navigation"
import type { PageLanding } from "./useGalleryKeyboard"
import { useElementSize } from "./useElementWidth"
import { useGalleryScroller, type GalleryScroller } from "./useGalleryScroller"

/**
 * 목차 레일(P5)이 갤러리를 읽고 움직이는 창구. 레일은 이 값만 알면 되고 블록·행 모델은 모른다.
 * 위치(fraction)는 두 모드 모두 "전체 결과 기준" 0..1 이다.
 */
export interface GalleryScrollController {
  mode: GalleryPaging
  total: number
  index: SearchIndex | undefined
  /** 현재 위치 0..1 — 무한: 오프셋 / 최대 오프셋, 페이지: 이 쪽이 전체에서 차지하는 구간 안의 위치 */
  fraction: number
  /** 0-기준 쪽 번호와 쪽 수 (무한 스크롤은 0 / 1) */
  page: number
  pageCount: number
  /** 지금 보여 주는 전역 범위 [start, end) */
  range: readonly [number, number]
  /** 레일 위치 → 이동. 무한: 오프셋 비율, 페이지: 그 비율의 항목이 든 쪽으로 이동 후 그 항목까지 스크롤. `jump` 거짓이면 빠른 스크롤로 보아 요청을 미룬다(드래그) */
  scrollToFraction: (fraction: number, opts?: { jump?: boolean }) => void
  /** 구간(`index.sections[i]`) 머리로 이동 */
  scrollToSection: (sectionIndex: number) => void
  /** 전역 항목 위치로 이동(그 행이 맨 위, 구간 첫 행이면 머리글까지) */
  scrollToItem: (item: number) => void
  /** 구간 `sectionIndex` 의 레일 위치 0..1 — `fraction` 과 같은 기준 */
  sectionFraction: (sectionIndex: number) => number
  /** 화면 맨 위 행의 첫 항목(전역 위치) — 구간 안 진행도(레일 핸들)와 현재 구간 판정용 */
  firstItem: number
  /** 화면에 (일부라도) 보이는 마지막 항목 — 끝 쪽 구간은 머리가 맨 위에 올 수 없어 "머리가 보이는가" 로 현재 구간을 판정한다 */
  lastItem: number
  /** 레일 위 휠을 뷰포트와 똑같이 처리 */
  wheel: (e: WheelEvent) => void
  /** 스크롤·이동이 있을 때마다 호출 (레일 자동 표시). 해제 함수를 돌려준다 */
  onScrollActivity: (cb: () => void) => () => void
}

export interface GalleryViewport {
  win: GalleryWindow
  geo: GalleryGeometry
  scroller: GalleryScroller
  nav: GalleryNav
  controller: GalleryScrollController
  /** 쪽 이동 (페이지 모드) — 도착하면 `landing` 항목이 보이게 스크롤 */
  goToPage: (page: number, landing?: PageLanding) => void
  /** 목적지 항목이 화면에 들어오게 최소 이동 (키보드 커서용) */
  reveal: (item: number) => void
  viewportW: number
  viewportH: number
}

interface Args {
  /** 휠을 받고 크기를 재는 요소 */
  el: HTMLElement | null
  paging: GalleryPaging
  pageSize: number
}

/**
 * 갤러리 데이터 창 + 배치 + 오프셋 스크롤을 한데 묶는다.
 * - 보이는 항목 구간을 블록 캐시에 알린다 (빠른 스크롤이면 멈출 때까지 요청 지연)
 * - 모드·쪽 크기·카드 크기·창 폭·색인 세대가 바뀌면 화면 첫 항목을 앵커로 위치를 복원한다
 * - 검색 조건이 바뀌어 새 결과가 오면 맨 위(1쪽)로
 */
export function useGalleryViewport({ el, paging, pageSize }: Args): GalleryViewport {
  const page = useViewStore((s) => s.galleryPage)
  const setPage = useViewStore((s) => s.setGalleryPage)
  // 필름스트립 중에도 숨은 갤러리는 직전 격자/목록 배치를 유지한다 (돌아왔을 때 위치·열 수가 그대로)
  const viewMode = useViewStore((s) => (s.viewMode === "filmstrip" ? s.prevViewMode : s.viewMode))
  const display = useViewStore((s) => s.cardDisplay)
  const win = useGalleryWindow({ mode: paging, pageSize, page })
  const { width, height } = useElementSize(el)
  const geo = useGalleryGeometry({ viewMode, width, viewportH: height, display, index: win.index, range: win.range, shownSortField: win.shownSortField })
  const scroller = useGalleryScroller({ el, contentH: geo.contentH, viewportH: height })
  const { offset, scrollTo, isFast } = scroller
  const { requestRange } = win

  // 보이는 항목 구간을 블록 캐시에 알린다
  useEffect(() => {
    const r = visibleItems(geo, offset)
    if (r) requestRange(r[0], r[1], { fast: isFast() })
  }, [geo, offset, requestRange, isFast])

  const pending = useRef<{ item: number; align: "top" | "end" } | null>(null)
  const scrollToApplied = (item: number, align: "top" | "end") => {
    const off = align === "end" ? offsetToReveal(geo, item, 0) : offsetOfItemTop(geo, item)
    if (off !== null) scrollTo(off, { jump: true })
  }
  /** 항목 위치로 이동. 페이지 모드에서 다른 쪽이면 쪽을 바꾼 뒤(다음 렌더) 스크롤한다 */
  const goToItem = (item: number, align: "top" | "end" = "top") => {
    if (win.total <= 0) return
    const g = Math.min(win.total - 1, Math.max(0, Math.floor(item)))
    if (paging === "paged" && Math.floor(g / pageSize) !== win.page) {
      pending.current = { item: g, align }
      setPage(Math.floor(g / pageSize))
      return
    }
    scrollToApplied(g, align)
  }
  const goToPage = (p: number, landing: PageLanding = "first") => {
    const target = Math.min(win.pageCount - 1, Math.max(0, p))
    const item = landing === "first" ? target * pageSize : Math.min(win.total, (target + 1) * pageSize) - 1
    if (target === win.page) scrollToApplied(Math.max(0, item), landing === "first" ? "top" : "end")
    else {
      pending.current = { item: Math.max(0, item), align: landing === "first" ? "top" : "end" }
      setPage(target)
    }
  }
  const reveal = (item: number) => {
    const off = offsetToReveal(geo, item, offset)
    if (off !== null) scrollTo(off, { jump: true })
  }

  // 위치 복원: 설정(모드·쪽 크기)·배치(보기·열 수·행 높이·머리글)·색인 세대가 바뀌면 직전의 첫 보이는 항목으로
  const anchor = useRef(0)
  const keys = useRef({ cfg: `${paging}|${pageSize}`, layout: "" })
  const appliedRequest = useRef(win.request)
  const cfgKey = `${paging}|${pageSize}`
  const layoutKey = `${viewMode}|${geo.cols}|${geo.rowH}|${geo.grouped}|${win.generation}`
  useLayoutEffect(() => {
    if (!win.isRefreshing && appliedRequest.current !== win.request) {
      // 새 검색 결과 도착 — 맨 위(1쪽)로, 옛 위치는 잊는다
      appliedRequest.current = win.request
      keys.current = { cfg: cfgKey, layout: layoutKey }
      anchor.current = 0
      pending.current = null
      setPage(0)
      scrollTo(0, { jump: true })
      return
    }
    const cfgChanged = keys.current.cfg !== cfgKey
    if (cfgChanged || keys.current.layout !== layoutKey) {
      const hadLayout = keys.current.layout !== ""
      keys.current = { cfg: cfgKey, layout: layoutKey }
      if (hadLayout && win.total > 0) {
        // 모드·쪽 크기 변경은 앵커가 든 쪽으로, 그 밖에는 지금 쪽 안에서 (세대 변경 후 총계가 줄었으면 범위 안으로)
        const inPage = Math.min(win.range[1] - 1, Math.max(win.range[0], anchor.current))
        goToItem(cfgChanged ? Math.min(win.total - 1, anchor.current) : inPage)
      }
    }
    const p = pending.current
    if (p) {
      if (p.item >= geo.model.rangeStart && p.item < geo.model.rangeEnd) {
        pending.current = null
        scrollToApplied(p.item, p.align)
      } else if (p.item >= win.total) pending.current = null
      return
    }
    const first = firstVisibleItem(geo, offset)
    if (first !== null) anchor.current = first
  })

  // 목차 레일 창구
  const starts = useMemo(() => {
    const out = [0]
    for (const s of win.index?.sections ?? []) out.push(out[out.length - 1] + s.count)
    return out
  }, [win.index])
  const fractionOfOffset = scroller.maxOffset > 0 ? offset / scroller.maxOffset : 0
  const controller: GalleryScrollController = {
    mode: paging,
    total: win.total,
    index: win.index,
    fraction: paging === "infinite" ? fractionOfOffset : win.total > 0 ? (win.range[0] + fractionOfOffset * (win.range[1] - win.range[0])) / win.total : 0,
    page: win.page,
    pageCount: win.pageCount,
    range: win.range,
    scrollToFraction: (f, o) => {
      const fr = Math.min(1, Math.max(0, f))
      if (paging === "infinite") scrollTo(fr * scroller.maxOffset, o)
      else goToItem(Math.floor(fr * win.total))
    },
    scrollToSection: (i) => goToItem(starts[i] ?? 0),
    scrollToItem: (item) => goToItem(item),
    sectionFraction: (i) => {
      if (paging === "paged") return win.total > 0 ? (starts[i] ?? 0) / win.total : 0
      // 구간 첫 항목의 스크롤 위치 — 묶음 배치(구간 머리글)와 묶지 않는 배치(구간 하나로 합쳐진 행 모델) 모두 항목 위치로 환산한다
      const off = offsetOfItemTop(geo, starts[i] ?? 0)
      return off !== null && scroller.maxOffset > 0 ? Math.min(1, off / scroller.maxOffset) : 0
    },
    onScrollActivity: scroller.subscribe,
    firstItem: firstVisibleItem(geo, offset) ?? win.range[0],
    lastItem: visibleItems(geo, offset)?.[1] ?? Math.max(win.range[0], win.range[1] - 1),
    wheel: scroller.handleWheel,
  }

  const rows = Math.max(1, Math.floor((height - (geo.kind === "list" ? geo.padTop : 0)) / geo.rowH))
  const nav = useMemo<GalleryNav>(() => ({ cols: geo.cols, pageSize: rows * geo.cols, horizontal: geo.kind === "grid" }), [geo.cols, geo.kind, rows])
  return { win, geo, scroller, nav, controller, goToPage, reveal, viewportW: width, viewportH: height }
}
