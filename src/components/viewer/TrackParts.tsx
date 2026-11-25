import type { CSSProperties, PointerEvent as ReactPointerEvent } from "react"
import type { SceneCut } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { LAYER_HEIGHT, buildCutPaths, trianglePath } from "./sceneLayer"

/** 가로 위치·폭 — 길이에 대한 비율이라 계산 값을 style 로 준다 */
export function spanStyle(startMs: number, endMs: number, durationMs: number): CSSProperties {
  return { left: `${(startMs / durationMs) * 100}%`, width: `${((endMs - startMs) / durationMs) * 100}%` }
}

/** 구간 박스 세로 자리 — 36px 트랙 가운데 22px */
const BOX_Y = "absolute top-1/2 h-[22px] -translate-y-1/2"

interface BoxProps {
  index: number
  selected: boolean
  style: CSSProperties
  label: string
  onStartHandle: (e: ReactPointerEvent) => void
  onEndHandle: (e: ReactPointerEvent) => void
}

/**
 * 확정 구간 박스 — 눌러도 탐색만 한다(포인터는 트랙으로 버블링). 선택은 우클릭·키로 하며 선택된 구간만 양끝 8px 핸들을 갖는다.
 * 접근성 이름(시각 포함)은 버튼에 둔다.
 */
export function RangeBox({ index, selected, style, label, onStartHandle, onEndHandle }: BoxProps) {
  return (
    <div data-range-index={index} className={BOX_Y} style={style}>
      <button
        type="button"
        tabIndex={-1}
        aria-label={label}
        aria-pressed={selected}
        className={cn("absolute inset-0 min-w-0.5 cursor-pointer rounded-sm border border-primary/80 bg-primary/30 outline-none", selected && "border-2 border-primary bg-primary/45")}
      />
      {selected && (
        <>
          <span aria-hidden="true" data-testid="range-handle-start" onPointerDown={onStartHandle} className="absolute -top-0.5 -bottom-0.5 -left-1 z-10 w-2 cursor-ew-resize rounded-sm bg-foreground" />
          <span aria-hidden="true" data-testid="range-handle-end" onPointerDown={onEndHandle} className="absolute -top-0.5 -right-1 -bottom-0.5 z-10 w-2 cursor-ew-resize rounded-sm bg-foreground" />
        </>
      )}
    </div>
  )
}

/** 끌어서 만드는 중인 새 구간 미리보기 */
export function CreatingBox({ startMs, endMs, durationMs }: { startMs: number; endMs: number; durationMs: number }) {
  return <div aria-hidden="true" className={cn(BOX_Y, "pointer-events-none rounded-sm border border-primary bg-primary/40")} style={spanStyle(startMs, endMs, durationMs)} />
}

/** 대기 In/Out — 표시한 지점에서 현재 위치까지의 점선 */
export function PendingSpan({ from, to, durationMs }: { from: number; to: number; durationMs: number }) {
  return (
    <div
      aria-hidden="true"
      data-testid="range-pending"
      className={cn(BOX_Y, "pointer-events-none min-w-0.5 rounded-sm border border-dashed border-primary bg-primary/10")}
      style={spanStyle(from, from + Math.max(to - from, 0), durationMs)}
    />
  )
}

/**
 * 트랙 위쪽 가장자리의 장면전환점 역삼각형 — 컷 수와 무관하게 SVG path 는 자동/사용자 2개뿐이다(컷마다 DOM 요소를 만들지 않는다).
 * 포인터를 받지 않고, 구간 박스·레일·썸 아래에 깔린다.
 */
export function CutTriangles({ cuts, durationMs, trackPx }: { cuts: readonly SceneCut[]; durationMs: number; trackPx: number }) {
  if (durationMs <= 0 || trackPx <= 0) return null
  const paths = buildCutPaths(cuts, durationMs, trackPx)
  return (
    <svg
      aria-hidden="true"
      data-testid="scene-cut-layer"
      viewBox={`0 0 ${trackPx} ${LAYER_HEIGHT}`}
      preserveAspectRatio="none"
      className="pointer-events-none absolute inset-x-0 top-0 w-full overflow-visible"
      height={LAYER_HEIGHT}
    >
      <path data-source="auto" d={paths.auto} className="fill-scene-cut-auto" />
      <path data-source="user" d={paths.user} className="fill-scene-cut-user" />
    </svg>
  )
}

/** hover 중인 컷만 크게 그리는 삼각형 */
export function HoverCut({ cut, durationMs }: { cut: SceneCut; durationMs: number }) {
  return (
    <svg aria-hidden="true" width="1" height="1" className="pointer-events-none absolute top-0 overflow-visible" style={{ left: `${(cut.ms / durationMs) * 100}%` }}>
      <path d={trianglePath(0, cut.source, 1.4)} className={cut.source === "auto" ? "fill-scene-cut-auto" : "fill-scene-cut-user"} />
    </svg>
  )
}

/** 시각 툴팁 — 플롯 왼쪽 끝 기준 `left` 위치 위에 뜬다 */
export function TrackTooltip({ left, children }: { left: string; children: string }) {
  return (
    <div
      role="tooltip"
      className="pointer-events-none absolute bottom-full z-30 mb-1 -translate-x-1/2 rounded-md bg-foreground px-2 py-1 text-xs whitespace-nowrap text-background"
      style={{ left }}
    >
      {children}
    </div>
  )
}
