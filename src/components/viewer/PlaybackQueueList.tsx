import { Brackets } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { useDisplayName } from "@/hooks/useDisplayName"
import { usePlayRangeSummary } from "@/hooks/usePlayRanges"
import { formatDuration } from "@/lib/format"
import type { PlayQueueItem } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { usePlaybackStore } from "@/stores"
import { useFocusExclusion } from "./useFocusExclusion"

/** 행 높이(px) — 고정이라 보이는 행만 계산해 그린다 */
const ROW_H = 28
/** 목록 영역에 한 번에 보이는 행 수 */
const VISIBLE_ROWS = 7
const OVERSCAN = 4

function stateLabelKey(item: PlayQueueItem, failed: boolean): string | null {
  if (failed) return "playback.failed"
  if (item.availability === "offline") return "playback.offline"
  if (item.availability === "missing") return "playback.missing"
  return null
}

/**
 * 연속 재생 대기열 — 현재 항목 강조, 건너뛰는 항목(오프라인·파일 없음·재생 오류)은 흐리게, 클릭하면 그 항목으로 이동.
 * 컬렉션이 커도 보이는 행만 그린다(고정 행 높이 창 계산) — 전체 항목을 DOM 에 올리지 않는다.
 */
export function PlaybackQueueList() {
  const { t } = useTranslation()
  const queue = usePlaybackStore((s) => s.queue)
  const index = usePlaybackStore((s) => s.index)
  const failed = usePlaybackStore((s) => s.failed)
  const collectionId = usePlaybackStore((s) => s.collectionId)
  // 구간이 있는 항목만 (희소) — 재생 중인 컬렉션마다 1회, 구간 편집 저장 때만 다시 받는다
  const summaries = usePlayRangeSummary(collectionId)
  const jumpTo = usePlaybackStore((s) => s.jumpTo)
  const { nameOf } = useDisplayName()
  const scroller = useRef<HTMLDivElement>(null)
  const [top, setTop] = useState(0)
  useFocusExclusion(scroller)

  // 현재 항목이 바뀌면 보이는 영역 안으로 (이미 보이면 그대로)
  useEffect(() => {
    const el = scroller.current
    if (!el || index < 0) return
    const y = index * ROW_H
    if (y < el.scrollTop || y + ROW_H > el.scrollTop + VISIBLE_ROWS * ROW_H) {
      el.scrollTop = Math.max(0, y - ROW_H * 2)
      setTop(el.scrollTop)
    }
  }, [index])

  const first = Math.max(0, Math.floor(top / ROW_H) - OVERSCAN)
  const last = Math.min(queue.length - 1, Math.floor(top / ROW_H) + VISIBLE_ROWS + OVERSCAN)

  return (
    <div
      ref={scroller}
      role="list"
      aria-label={t("playback.queue")}
      onScroll={(e) => setTop(e.currentTarget.scrollTop)}
      className="relative max-h-56 min-h-0 shrink-0 overflow-y-auto border-t border-border"
    >
      <div className="relative" style={{ height: queue.length * ROW_H }}>
        {Array.from({ length: Math.max(0, last - first + 1) }, (_, k) => {
          const i = first + k
          const item = queue[i]
          const isFailed = failed.has(item.assetId)
          const stateKey = stateLabelKey(item, isFailed)
          const summary = item.mediaType === "image" ? undefined : summaries.get(item.assetId)
          return (
            <div key={item.assetId} role="listitem" className="absolute inset-x-0" style={{ top: i * ROW_H, height: ROW_H }}>
              <button
                type="button"
                aria-current={i === index ? "true" : undefined}
                onClick={() => jumpTo(item.assetId)}
                className={cn(
                  "flex size-full items-center gap-2 px-3 text-left text-xs hover:bg-muted",
                  i === index && "bg-accent font-medium text-accent-foreground",
                  stateKey !== null && "opacity-50"
                )}
              >
                <span className="w-8 shrink-0 text-right text-muted-foreground tabular-nums">{i + 1}</span>
                <span className="min-w-0 flex-1 truncate">{nameOf(item)}</span>
                {summary !== undefined && (
                  <span className="flex shrink-0 items-center gap-1 text-muted-foreground tabular-nums">
                    <Brackets aria-hidden="true" className="size-3" />
                    {t("ranges.summary", { count: summary.rangeCount, total: formatDuration(summary.totalMs) })}
                  </span>
                )}
                <span className="shrink-0 text-muted-foreground tabular-nums">
                  {stateKey !== null ? t(stateKey) : item.mediaType === "image" ? "" : formatDuration(item.durationMs)}
                </span>
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
}
