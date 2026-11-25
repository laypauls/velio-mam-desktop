import { memo, useCallback, useEffect, useMemo, useRef, useState, type MouseEvent } from "react"
import { useTranslation } from "react-i18next"
import { formatNumber } from "@/lib/format"
import type { IndexRailMode } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { buildTicks, handleFraction, sectionAtFraction, sectionOfItem, sectionText, slotAt, slotFraction, type RailTicks } from "./railModel"
import { useElementSize } from "./useElementWidth"
import type { GalleryScrollController } from "./useGalleryViewport"

/** 스크롤이 멈추고 라벨이 사라질 때까지 */
export const RAIL_IDLE_MS = 1500
/** 이 거리(px) 이상 움직이면 클릭이 아니라 드래그 */
export const DRAG_THRESHOLD_PX = 3
const THUMB_H = 28
const BUBBLE_HALF = 14

const clamp01 = (n: number) => Math.min(1, Math.max(0, n))

/** 라벨·점 — 위치가 같으면 다시 그리지 않도록 분리 */
const RailTicksView = memo(function RailTicksView({ ticks, visible }: { ticks: RailTicks; visible: boolean }) {
  return (
    <div
      aria-hidden="true"
      data-testid="rail-ticks"
      data-state={visible ? "visible" : "hidden"}
      className="pointer-events-none absolute inset-0 opacity-0 transition-opacity duration-200 data-[state=visible]:opacity-100"
    >
      {ticks.dots.map((y, i) => (
        <i key={i} style={{ top: y }} className="absolute right-2.5 size-0.5 -translate-y-1/2 rounded-full bg-muted-foreground/60" />
      ))}
      {ticks.labels.map((l) => (
        <span key={`${l.key}`} style={{ top: l.y }} className="absolute right-4 -translate-y-1/2 whitespace-nowrap text-[10.5px] font-semibold leading-none text-muted-foreground">
          {l.text}
        </span>
      ))}
    </div>
  )
})

interface Props {
  controller: GalleryScrollController
  mode: IndexRailMode
  /** 페이지 모드의 한 쪽 크기 — 레일 위치 → 쪽 번호 환산 */
  pageSize: number
  /** 이 레일이 조작하는 갤러리 스크롤 영역의 id (`aria-controls`) */
  scrollAreaId?: string
}

/**
 * 갤러리 오른쪽 목차 레일 + 위치 핸들.
 * - 핸들은 항상 보이고, 목차 라벨·점은 스크롤·호버·드래그 중에 나타났다가 1.5초 뒤 사라진다 (`always` 는 유지, `off` 는 라벨 없음)
 * - 클릭 = 구간 머리로 이동, 드래그 = 무한: 연속 이동 / 페이지: 말풍선 미리보기만 하다가 놓을 때 이동
 * - 포커스를 받지 않는다(Tab 제외, 클릭·드래그도 기존 포커스 유지) — 키보드 이동은 갤러리가 맡는다
 * 위치·구간 정보는 모두 `controller` 에서 읽는다 — 블록·행 모델은 모른다.
 */
export function IndexRail({ controller, mode, pageSize, scrollAreaId }: Props) {
  const { t } = useTranslation()
  const { index, total, fraction, mode: paging, range, onScrollActivity, firstItem } = controller
  const [track, setTrack] = useState<HTMLDivElement | null>(null)
  const { height: trackH } = useElementSize(track)

  // 표시 상태
  const [active, setActive] = useState(false)
  const [hoverF, setHoverF] = useState<number | null>(null)
  const [dragF, setDragF] = useState<number | null>(null)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const flash = useCallback(() => {
    setActive(true)
    clearTimeout(timer.current)
    timer.current = setTimeout(() => setActive(false), RAIL_IDLE_MS)
  }, [])
  useEffect(() => onScrollActivity(flash), [onScrollActivity, flash])
  useEffect(() => () => clearTimeout(timer.current), [])

  const sections = index?.sections
  const kind = index?.kind ?? "none"
  const initial = kind === "initial"
  // 구간 시작 위치(누적) — 이름 목차의 칸·핸들 보간·현재 구간 판정에 쓴다
  const starts = useMemo(() => {
    const out = [0]
    for (const sec of sections ?? []) out.push(out[out.length - 1] + sec.count)
    return out
  }, [sections])
  const nSections = sections?.length ?? 0
  // 이름 목차는 개수와 무관하게 칸을 균등 배치하므로 스크롤 위치 환산(fracs)이 필요 없다
  const fracs = useMemo(() => (initial ? [] : (sections ?? []).map((_, i) => controller.sectionFraction(i))), [controller, sections, initial])
  // controller 는 렌더마다 새로 만들어지므로 값이 같은 위치 배열은 문자열로 비교해 라벨을 다시 만들지 않는다
  const fp = fracs.map((f) => f.toFixed(4)).join(",")
  const stableFracs = useMemo(() => fracs, [fp])
  const ticks = useMemo(() => (mode === "off" ? { labels: [], dots: [] } : buildTicks(index, stableFracs, trackH, t)), [mode, index, stableFracs, trackH, t])

  const dragging = dragF !== null
  const hovering = hoverF !== null
  const visible = mode === "always" || active || hovering || dragging

  // 이벤트 핸들러(마우스 업 등)가 렌더 사이에 낡지 않게 최신 값을 ref 로
  const live = useRef({ controller, fracs, trackH, kind, starts, nSections })
  live.current = { controller, fracs, trackH, kind, starts, nSections }
  const rootRef = useRef<HTMLDivElement | null>(null)
  const [root, setRoot] = useState<HTMLDivElement | null>(null)
  const setRootEl = useCallback((el: HTMLDivElement | null) => {
    rootRef.current = el
    setRoot(el)
  }, [])
  // 레일 위 휠 = 뷰포트 휠과 같은 처리(컨트롤러가 같은 함수를 넘긴다). 페이지가 같이 스크롤되지 않게 비수동(non-passive) 으로 받는다
  useEffect(() => {
    if (!root) return
    const onWheel = (e: WheelEvent) => {
      live.current.controller.wheel(e)
      flash()
    }
    root.addEventListener("wheel", onWheel, { passive: false })
    return () => root.removeEventListener("wheel", onWheel)
  }, [root, flash])
  const anchor = useRef<number | null>(null) // 마지막으로 레일이 보낸 구간 — 위치 오차로 같은 구간에 머무는 것을 막는다
  const detach = useRef<(() => void) | null>(null)
  useEffect(() => () => detach.current?.(), [])
  useEffect(() => {
    anchor.current = null // 색인이 바뀌면 옛 구간 번호는 무효
  }, [index])

  const fracAt = (clientY: number) => {
    const r = track?.getBoundingClientRect()
    const h = r?.height || live.current.trackH
    return h > 0 && r ? clamp01((clientY - r.top) / h) : 0
  }
  /** 이름 목차: 트랙 비율 → 가장 가까운 칸의 구간 */
  const slotSection = (f: number) => slotAt(live.current.nSections, f, live.current.trackH)
  const pageOfItem = (g: number) => Math.min(controller.pageCount - 1, Math.max(0, Math.floor(g / Math.max(1, pageSize))))
  const pageOf = (f: number) => Math.min(controller.pageCount - 1, Math.max(0, Math.floor(f * total / Math.max(1, pageSize))))
  /** 이름 목차의 "현재 위치 항목" — 레일이 보낸 구간의 머리가 화면에 보이면(머리 행에 앞 구간 항목이 섞이거나, 끝 쪽이라 머리가 맨 위에 못 오는 경우) 그 구간 시작으로 본다 */
  const effectiveFirst = () => {
    const { starts: st, controller: c } = live.current
    const a0 = anchor.current
    return a0 !== null && st[a0 + 1] !== undefined && c.firstItem <= st[a0] && st[a0] <= c.lastItem ? st[a0] : c.firstItem
  }
  const jumpToSection = (i: number) => {
    anchor.current = i
    live.current.controller.scrollToSection(i)
  }

  const onMouseDown = (e: MouseEvent<HTMLDivElement>) => {
    if (e.button !== 0) return
    e.preventDefault() // 포커스를 가져가지 않는다 — 갤러리의 키보드 포커스를 그대로 둔다
    const startY = e.clientY
    let moved = false
    let dragSlot = -1
    const onMove = (ev: globalThis.MouseEvent) => {
      if (!moved && Math.abs(ev.clientY - startY) < DRAG_THRESHOLD_PX) return
      moved = true
      const f = fracAt(ev.clientY)
      setDragF(f)
      setHoverF(f)
      // 무한 스크롤: 끄는 동안 연속 이동(빠른 스크롤이라 블록 요청은 멈출 때까지 미뤄진다). 페이지: 미리보기만
      if (live.current.controller.mode !== "infinite") return
      if (live.current.kind === "initial") {
        // 이름 목차: 포인터 아래 구간이 바뀔 때만 그 구간 머리로
        const i = slotSection(f)
        if (i !== dragSlot) {
          dragSlot = i
          jumpToSection(i)
        }
      } else live.current.controller.scrollToFraction(f, { jump: false })
    }
    const end = () => {
      window.removeEventListener("mousemove", onMove)
      window.removeEventListener("mouseup", onUp)
      detach.current = null
    }
    const onUp = (ev: globalThis.MouseEvent) => {
      end()
      const f = fracAt(ev.clientY)
      const c = live.current.controller
      setDragF(null)
      if (live.current.kind === "initial") {
        // 이름 목차: 클릭이든 드래그든 놓은 자리의 구간으로 (무한 스크롤 드래그 중 이미 이동했으면 다시 보내지 않는다)
        const i = slotSection(f)
        if (!(moved && c.mode === "infinite" && i === dragSlot)) jumpToSection(i)
      } else if (moved) {
        anchor.current = null
        c.scrollToFraction(f, { jump: true })
      } else if (live.current.kind === "none") {
        anchor.current = null
        c.scrollToFraction(f, { jump: true })
      } else jumpToSection(sectionAtFraction(live.current.fracs, f))
      if (rootRef.current?.matches(":hover") !== true) setHoverF(null)
      flash()
    }
    detach.current = end
    window.addEventListener("mousemove", onMove)
    window.addEventListener("mouseup", onUp)
  }

  if (total <= 0) return null

  // 표시값. 이름 목차는 포인터를 가장 가까운 칸에 붙이고, 핸들은 구간 안 진행도로 칸 사이를 보간한다
  const headItem = initial ? effectiveFirst() : firstItem
  const handleF = initial ? handleFraction(starts, headItem, trackH) : fraction
  const pointerF = dragF ?? hoverF
  const slot = initial && pointerF !== null ? slotAt(nSections, pointerF, trackH) : -1
  const f = initial && slot >= 0 ? slotFraction(nSections, slot, trackH) : (pointerF ?? fraction)
  const bubbleSection = sections ? sections[initial ? (slot >= 0 ? slot : sectionOfItem(starts, headItem)) : sectionAtFraction(stableFracs, f)] : undefined
  const bubbleItem = initial && slot >= 0 ? starts[slot] : null
  const pageText =
    paging === "paged" ? t("gallery.pager.page", { page: formatNumber((bubbleItem !== null ? pageOfItem(bubbleItem) : pageOf(f)) + 1) }) : null
  const headline = bubbleSection && kind !== "none" ? sectionText(kind, bubbleSection, t) : t("gallery.rail.percent", { n: Math.round(f * 100) })
  const detail = [kind !== "none" && bubbleSection ? t("gallery.rail.count", { n: formatNumber(bubbleSection.count) }) : null, pageText].filter(Boolean).join(" · ")

  const curSection = sections?.[initial ? sectionOfItem(starts, headItem) : sectionAtFraction(stableFracs, fraction)]
  const curText = curSection && kind !== "none" ? sectionText(kind, curSection, t) : null
  const valueText = [curText ?? t("gallery.rail.percent", { n: Math.round(fraction * 100) }), paging === "paged" ? t("gallery.pager.page", { page: formatNumber(controller.page + 1) }) : null]
    .filter(Boolean)
    .join(" · ")

  const thumbF = paging === "paged" && dragF !== null ? (initial ? slotFraction(nSections, slotAt(nSections, dragF, trackH), trackH) : dragF) : handleF
  const thumbY = Math.min(trackH - THUMB_H / 2, Math.max(THUMB_H / 2, thumbF * trackH))
  const bubbleY = Math.min(trackH - BUBBLE_HALF, Math.max(BUBBLE_HALF, f * trackH))
  const spanTop = initial ? handleFraction(starts, range[0], trackH) : range[0] / total
  const spanEnd = initial ? handleFraction(starts, Math.max(range[0], range[1] - 1), trackH) : range[1] / total
  const span = paging === "paged" && total > 0 ? { top: spanTop * trackH, height: Math.max(3, (spanEnd - spanTop) * trackH) } : null

  return (
    <div
      ref={setRootEl}
      role="scrollbar"
      aria-controls={scrollAreaId}
      aria-orientation="vertical"
      aria-label={t("gallery.rail.label")}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(handleF * 100)}
      aria-valuetext={valueText}
      onMouseDown={onMouseDown}
      onMouseMove={(e) => setHoverF(fracAt(e.clientY))}
      onMouseLeave={() => {
        if (dragF !== null) return
        setHoverF(null)
        flash()
      }}
      className="relative w-14 shrink-0 cursor-pointer select-none rounded-md"
    >
      <div ref={setTrack} className="absolute inset-y-1.5 right-0 left-0">
        <div className={cn("absolute inset-y-0 right-2 w-1 rounded-full transition-colors duration-200", visible ? "bg-muted" : "bg-transparent")} />
        {span && (
          <div
            data-testid="rail-page-span"
            style={{ top: span.top, height: span.height }}
            className={cn("absolute right-1.5 w-2 rounded-sm border border-primary bg-primary/20 transition-opacity duration-200", visible ? "opacity-100" : "opacity-0")}
          />
        )}
        <RailTicksView ticks={ticks} visible={visible} />
        {hoverF !== null && <div style={{ top: (initial ? f : hoverF) * trackH }} className="pointer-events-none absolute right-1 h-0.5 w-3 bg-primary" />}
        <div
          aria-hidden="true"
          style={{ top: thumbY - THUMB_H / 2, height: THUMB_H }}
          className="pointer-events-none absolute right-1.5 w-1.5 rounded-full bg-primary/70"
        />
        {(hovering || dragging) && (
          <div
            data-testid="rail-bubble"
            aria-hidden="true"
            style={{ top: bubbleY }}
            className="pointer-events-none absolute right-6 z-10 -translate-y-1/2 rounded-md border border-border bg-popover px-2 py-1 text-xs font-semibold whitespace-nowrap text-popover-foreground shadow-md"
          >
            {headline}
            {detail && <small className="ml-1.5 font-normal text-muted-foreground">{detail}</small>}
          </div>
        )}
      </div>
    </div>
  )
}
