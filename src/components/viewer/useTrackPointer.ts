import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react"
import type { PlayRange } from "@/lib/ipc-contract"
import type { RangeUi } from "./useRangeController"

/** 이 거리(px) 미만의 Shift+끌기는 새 구간으로 보지 않는다 */
const CLICK_PX = 3

export type TrackDrag =
  | { kind: "seek" }
  | { kind: "create"; anchorMs: number; atMs: number }
  | { kind: "start" | "end"; index: number; atMs: number }

/** 포인터를 올려 둔 위치 — `px` 는 플롯(썸 중심이 움직이는 폭) 왼쪽 끝 기준 */
export interface TrackHover {
  px: number
  ms: number
  shift: boolean
}

interface Options {
  /** 플롯 요소 — 좌표 계산 기준 (트랙 좌우 썸 반지름만큼 안쪽) */
  plotRef: RefObject<HTMLDivElement | null>
  durationMs: number
  ranges?: RangeUi
  onSeek: (seconds: number) => void
}

function capture(e: ReactPointerEvent, on: boolean) {
  const el = e.currentTarget
  try {
    if (on) el.setPointerCapture(e.pointerId)
    else el.releasePointerCapture(e.pointerId)
  } catch {
    // 이미 해제됐거나 지원하지 않는 환경 — 이동·해제 이벤트는 버블링으로도 트랙에 닿는다
  }
}

/** 핸들을 끌어도 반대쪽 경계를 넘거나 최소 길이보다 짧아지지 않게 맞춘 구간 */
export function adjusted(r: PlayRange, kind: "start" | "end", atMs: number, minMs: number): PlayRange {
  return kind === "start" ? { startMs: Math.min(atMs, r.endMs - minMs), endMs: r.endMs } : { startMs: r.startMs, endMs: Math.max(atMs, r.startMs + minMs) }
}

/**
 * 통합 트랙의 포인터 동작 — 눌러 끌기 = 탐색(끄는 동안 실시간), Shift+끌기 = 새 구간, 선택 구간 핸들 끌기 = 경계 조정, 그냥 올리기 = hover 위치.
 * 구간 끌기는 로컬 미리보기만 바꾸고 놓을 때 한 번만 `create`/`adjust` 로 확정한다(저장은 그쪽에서 디바운스).
 * 주 버튼이 아닌 누름은 아무것도 시작하지 않는다 — 우클릭은 재생 위치를 옮기지 않는다.
 * 이동·해제 이벤트는 포인터 캡처된 요소에서 트랙으로 버블링되므로 트랙 요소에 달린 핸들러가 받는다.
 */
export function useTrackPointer({ plotRef, durationMs, ranges, onSeek }: Options) {
  const [drag, setDrag] = useState<TrackDrag | null>(null)
  const [hover, setHover] = useState<TrackHover | null>(null)
  const live = useRef<TrackDrag | null>(null)
  const lastSeek = useRef<number | null>(null)
  const hoverRef = useRef<TrackHover | null>(null)
  const hovering = hover !== null
  const list: readonly PlayRange[] = ranges?.ranges ?? []
  const minMs = ranges?.minMs ?? 0

  const set = (d: TrackDrag | null) => {
    live.current = d
    setDrag(d)
  }
  const setHovered = (h: TrackHover | null) => {
    hoverRef.current = h
    setHover(h)
  }
  const plotX = (clientX: number): { px: number; width: number } => {
    const rect = plotRef.current?.getBoundingClientRect()
    if (!rect || rect.width <= 0) return { px: 0, width: 0 }
    return { px: Math.min(rect.width, Math.max(0, clientX - rect.left)), width: rect.width }
  }
  const msAt = (clientX: number): number => {
    const { px, width } = plotX(clientX)
    return width > 0 && durationMs > 0 ? Math.round((px / width) * durationMs) : 0
  }
  const seekTo = (ms: number) => {
    const seconds = Math.round(ms / 100) / 10
    if (lastSeek.current === seconds) return
    lastSeek.current = seconds
    onSeek(seconds)
  }

  // Shift 를 hover 중에 눌렀다 떼도 툴팁("· 새 구간")이 바로 따라가게 한다
  useEffect(() => {
    if (!hovering) return
    const sync = (e: KeyboardEvent) => {
      const h = hoverRef.current
      if (e.key === "Shift" && h && h.shift !== e.shiftKey) setHovered({ ...h, shift: e.shiftKey })
    }
    window.addEventListener("keydown", sync)
    window.addEventListener("keyup", sync)
    return () => {
      window.removeEventListener("keydown", sync)
      window.removeEventListener("keyup", sync)
    }
  }, [hovering])

  const onPointerDown = (e: ReactPointerEvent) => {
    if (e.button !== 0 || durationMs <= 0) return
    setHovered(null)
    const ms = msAt(e.clientX)
    if (e.shiftKey && ranges) {
      set({ kind: "create", anchorMs: ms, atMs: ms })
    } else {
      // 빈 곳을 누르면 선택 해제 — 구간 박스 위는 탐색만 한다
      const onBox = e.target instanceof Element && e.target.closest("[data-range-index]") !== null
      if (ranges && !onBox) ranges.select(null)
      lastSeek.current = null
      set({ kind: "seek" })
      seekTo(ms)
    }
    capture(e, true)
  }
  const onHandlePointerDown = (kind: "start" | "end", index: number) => (e: ReactPointerEvent) => {
    if (e.button !== 0) return
    e.stopPropagation()
    const r = list[index]
    set({ kind, index, atMs: kind === "start" ? r.startMs : r.endMs })
    capture(e, true)
  }
  const onPointerMove = (e: ReactPointerEvent) => {
    const d = live.current
    if (d) {
      const ms = msAt(e.clientX)
      if (d.kind === "seek") seekTo(ms)
      else set({ ...d, atMs: ms })
      return
    }
    if (e.buttons !== 0 || durationMs <= 0) return setHovered(null)
    setHovered({ px: plotX(e.clientX).px, ms: msAt(e.clientX), shift: e.shiftKey })
  }
  const finish = (e: ReactPointerEvent, commit: boolean) => {
    const d = live.current
    if (!d) return
    set(null)
    capture(e, false)
    if (!commit || d.kind === "seek") return
    if (d.kind === "create") {
      const { width } = plotX(e.clientX)
      const moved = width > 0 ? (Math.abs(d.atMs - d.anchorMs) / durationMs) * width : 0
      if (moved < CLICK_PX) return
      return ranges?.create({ startMs: Math.min(d.anchorMs, d.atMs), endMs: Math.max(d.anchorMs, d.atMs) })
    }
    const r = list[d.index]
    const next = adjusted(r, d.kind, d.atMs, minMs)
    if (next.startMs !== r.startMs || next.endMs !== r.endMs) ranges?.adjust(d.index, next)
  }

  /** 끌기 반영 후 그릴 구간들과 새 구간 미리보기 */
  const preview = {
    ranges: list.map((r, i) => (drag && (drag.kind === "start" || drag.kind === "end") && drag.index === i ? adjusted(r, drag.kind, drag.atMs, minMs) : r)),
    creating: drag?.kind === "create" ? { startMs: Math.min(drag.anchorMs, drag.atMs), endMs: Math.max(drag.anchorMs, drag.atMs) } : null,
  }

  return {
    preview,
    hover: drag ? null : hover,
    dragging: drag !== null,
    handlers: {
      onPointerDown,
      onPointerMove,
      onPointerUp: (e: ReactPointerEvent) => finish(e, true),
      onPointerCancel: (e: ReactPointerEvent) => finish(e, false),
      onPointerLeave: () => setHovered(null),
    },
    onHandlePointerDown,
  }
}
