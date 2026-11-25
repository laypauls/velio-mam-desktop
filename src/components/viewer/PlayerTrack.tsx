import { useRef, useState, type MouseEvent as ReactMouseEvent } from "react"
import { useTranslation } from "react-i18next"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import { formatDuration } from "@/lib/format"
import type { SceneCut } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { formatRangeTime } from "./playRanges"
import { formatSceneTime, nearestCut } from "./sceneLayer"
import { CreatingBox, CutTriangles, HoverCut, PendingSpan, RangeBox, TrackTooltip, spanStyle } from "./TrackParts"
import { TrackDialogs, TrackMenuContent, type TrackTarget } from "./TrackMenu"
import type { RangeUi } from "./useRangeController"
import type { SceneUi } from "./useSceneController"
import { useElementWidth } from "./useElementWidth"
import { useTrackPointer } from "./useTrackPointer"

interface Props {
  currentSec: number
  durationSec: number
  onSeek: (seconds: number) => void
  /** 없으면(구간 UI 없음) 구간 박스·Shift+끌기·구간 메뉴를 쓰지 않는다 */
  ranges?: RangeUi
  /** 없으면(음성 등) 장면전환점 삼각형·메뉴를 쓰지 않는다 */
  scene?: SceneUi
  /** 시작 전(포스터) — 탐색 불가 */
  disabled?: boolean
}

/**
 * 통합 트랙 — 탐색 슬라이더·구간 박스·장면전환점 삼각형을 36px 클릭 영역 하나에 합친다(높이는 hover 에도 고정 — libVLC 창이 출렁이지 않게).
 * 눌러 끌기 = 탐색(구간 박스 위도 동일), Shift+끌기 = 새 구간, 선택 구간 핸들 끌기 = 경계 조정, 우클릭 = 구간·장면 통합 메뉴(구간 선택 포함).
 * 접근성 slider 는 레일에 두고 구간 박스(버튼)는 형제로 둔다. 키보드 탐색은 플레이어 단축키가 맡으므로 이 영역은 포커스를 받지 않는다.
 */
export function PlayerTrack({ currentSec, durationSec, onSeek, ranges, scene, disabled = false }: Props) {
  const { t } = useTranslation()
  const plotRef = useRef<HTMLDivElement>(null)
  const trackPx = useElementWidth(plotRef)
  const [target, setTarget] = useState<TrackTarget>({ rangeIndex: null, cut: null })
  const enabled = durationSec > 0 && !disabled
  const durationMs = durationSec * 1000
  const max = enabled ? durationSec : 1
  const now = Math.min(Math.max(currentSec, 0), max)
  const { preview, hover, handlers, onHandlePointerDown } = useTrackPointer({ plotRef, durationMs, ranges, onSeek })
  const showCuts = scene?.visible === true
  const hasMenu = ranges !== undefined || scene !== undefined

  const cutAt = (px: number, width: number): SceneCut | null => (scene && showCuts ? nearestCut(scene.cuts, scene.durationMs, width, px) : null)
  const onContextMenu = (e: ReactMouseEvent) => {
    const el = e.target instanceof Element ? e.target.closest<HTMLElement>("[data-range-index]") : null
    const rangeIndex = el ? Number(el.dataset.rangeIndex) : null
    if (rangeIndex !== null) ranges?.select(rangeIndex)
    const rect = plotRef.current?.getBoundingClientRect()
    setTarget({ rangeIndex, cut: rect ? cutAt(e.clientX - rect.left, rect.width) : null })
  }

  const hoverCut = hover ? cutAt(hover.px, trackPx) : null
  const pendingIn = ranges?.pendingInMs ?? null
  const pendingOut = ranges?.pendingOutMs ?? null
  const currentMs = now * 1000
  const track = (
    <div
      data-testid="player-track"
      aria-disabled={!enabled || undefined}
      onContextMenu={onContextMenu}
      {...(enabled ? handlers : {})}
      className={cn("group/track relative h-9 touch-none select-none", enabled ? "cursor-pointer" : "opacity-50")}
    >
      <div ref={plotRef} className="absolute inset-y-0 right-[9px] left-[9px]">
        <div
          role="slider"
          tabIndex={-1}
          aria-label={t("viewer.seek")}
          aria-valuemin={0}
          aria-valuemax={max}
          aria-valuenow={now}
          aria-valuetext={t("viewer.position", { current: formatDuration(now * 1000), total: formatDuration(durationMs) })}
          aria-disabled={!enabled || undefined}
          className="absolute inset-0"
        >
          <div className="absolute inset-x-0 top-1/2 h-2 -translate-y-1/2 rounded-full bg-muted transition-[height] group-hover/track:h-2.5" />
          <div
            className="absolute top-1/2 left-0 h-2 -translate-y-1/2 rounded-full bg-primary transition-[height] group-hover/track:h-2.5"
            style={{ width: `${enabled ? (now / durationSec) * 100 : 0}%` }}
          />
        </div>
        {enabled && (
          <>
            {showCuts && scene && <CutTriangles cuts={scene.cuts} durationMs={scene.durationMs} trackPx={trackPx} />}
            {preview.ranges.map((r, i) => (
              <RangeBox
                key={`${r.startMs}-${r.endMs}-${i}`}
                index={i}
                selected={ranges?.selected === i}
                style={spanStyle(r.startMs, r.endMs, durationMs)}
                label={t("ranges.item", { n: i + 1, start: formatRangeTime(r.startMs), end: formatRangeTime(r.endMs) })}
                onStartHandle={onHandlePointerDown("start", i)}
                onEndHandle={onHandlePointerDown("end", i)}
              />
            ))}
            {preview.creating && <CreatingBox startMs={preview.creating.startMs} endMs={preview.creating.endMs} durationMs={durationMs} />}
            {pendingIn !== null && <PendingSpan from={pendingIn} to={Math.max(currentMs, pendingIn)} durationMs={durationMs} />}
            {pendingOut !== null && <PendingSpan from={Math.min(currentMs, pendingOut)} to={pendingOut} durationMs={durationMs} />}
            {hover && !hoverCut && <div aria-hidden="true" className="pointer-events-none absolute inset-y-1 w-px bg-foreground/50" style={{ left: hover.px }} />}
            <div
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 z-20 size-4.5 -translate-x-1/2 -translate-y-1/2 rounded-full bg-primary ring-2 ring-card"
              style={{ left: `${(now / durationSec) * 100}%` }}
            />
            {hoverCut && scene && (
              <>
                <HoverCut cut={hoverCut} durationMs={scene.durationMs} />
                <TrackTooltip left={`${(hoverCut.ms / scene.durationMs) * 100}%`}>
                  {t("scene.tooltip", { time: formatSceneTime(hoverCut.ms), source: t(hoverCut.source === "auto" ? "scene.sourceAuto" : "scene.sourceUser") })}
                </TrackTooltip>
              </>
            )}
            {hover && !hoverCut && <TrackTooltip left={`${hover.px}px`}>{formatDuration(hover.ms) + (hover.shift && ranges ? t("ranges.newRangeSuffix") : "")}</TrackTooltip>}
          </>
        )}
      </div>
    </div>
  )

  return (
    <>
      {hasMenu ? (
        <ContextMenu>
          <ContextMenuTrigger asChild>{track}</ContextMenuTrigger>
          <TrackMenuContent ranges={ranges} scene={scene} target={target} />
        </ContextMenu>
      ) : (
        track
      )}
      {ranges && (pendingIn !== null || pendingOut !== null) && (
        <span role="status" className="sr-only">
          {pendingIn !== null ? t("ranges.pendingIn", { time: formatRangeTime(pendingIn) }) : t("ranges.pendingOut", { time: formatRangeTime(pendingOut ?? 0) })}
        </span>
      )}
      {ranges?.saveFailed && (
        <p role="alert" className="mt-0.5 text-xs text-destructive break-keep">
          {t("ranges.saveFailed")}
        </p>
      )}
      <TrackDialogs ranges={ranges} scene={scene} />
    </>
  )
}
