import { Brackets, Pause, Play, Repeat, SkipBack, SkipForward } from "lucide-react"
import { useRef } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { useSettings, useUpdateSettings } from "@/hooks/useSettings"
import { usePlaybackStore } from "@/stores"
import { useFocusExclusion } from "./useFocusExclusion"
import { usePlaybackKeys } from "./usePlaybackKeys"
import { useSlideshowTimer } from "./useSlideshowTimer"

/** 설정을 아직 못 받았을 때의 이미지 표시 시간(초) — 설정 기본값과 같다 */
const DEFAULT_SLIDESHOW_SECONDS = 5

/**
 * 연속 재생 컨트롤 — ⏮ / ⏸(이미지) / ⏭, 이미지 표시 진행, `n / 총수`, 반복 토글.
 * 영상·음성의 일시정지·탐색은 기존 컨트롤 바(`MediaControls`)가 맡는다. 재생이 활성인 동안만 마운트된다.
 */
export function PlaybackBar() {
  const { t } = useTranslation()
  const status = usePlaybackStore((s) => s.status)
  const index = usePlaybackStore((s) => s.index)
  const total = usePlaybackStore((s) => s.queue.length)
  const token = usePlaybackStore((s) => s.token)
  const repeat = usePlaybackStore((s) => s.repeat)
  const isImage = usePlaybackStore((s) => s.queue[s.index]?.mediaType === "image")
  const { next, prev, togglePause, toggleRepeat } = usePlaybackStore.getState()
  const settings = useSettings().data
  const updateSettings = useUpdateSettings({ silent: true })
  const seconds = settings?.slideshowImageSeconds ?? DEFAULT_SLIDESHOW_SECONDS
  const rangesOnly = settings?.collectionPlayRangesOnly === true
  const root = useRef<HTMLDivElement>(null)
  useFocusExclusion(root)
  usePlaybackKeys()
  const progress = useSlideshowTimer(isImage && status === "playing", seconds, token, next)
  const paused = status === "paused"
  const stopped = status === "stopped"

  return (
    <div ref={root} role="group" aria-label={t("playback.bar")} className="flex flex-col gap-1.5 border-t border-border px-3 py-2">
      {isImage && !stopped && <Progress value={progress * 100} aria-label={t("playback.imageCountdown", { seconds })} className="h-1" />}
      <div className="flex items-center gap-1">
        <Button size="icon-sm" variant="ghost" aria-label={t("playback.prev")} disabled={index <= 0 && repeat === "off"} onClick={prev}>
          <SkipBack aria-hidden="true" />
        </Button>
        {isImage && !stopped && (
          <Button size="icon-sm" variant="ghost" aria-label={t(paused ? "playback.resume" : "playback.pause")} onClick={togglePause}>
            {paused ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}
          </Button>
        )}
        <Button size="icon-sm" variant="ghost" aria-label={t("playback.next")} onClick={next}>
          <SkipForward aria-hidden="true" />
        </Button>
        <Button
          size="sm"
          variant={rangesOnly ? "secondary" : "ghost"}
          aria-pressed={rangesOnly}
          title={t("playback.rangesOnlyHint")}
          disabled={settings === undefined}
          onClick={() => updateSettings.mutate({ collectionPlayRangesOnly: !rangesOnly })}
        >
          <Brackets aria-hidden="true" />
          {t("ranges.only")}
        </Button>
        <span role="status" className="min-w-0 flex-1 truncate text-center text-xs text-muted-foreground tabular-nums">
          {stopped ? t("playback.stopped") : t("playback.position", { n: index + 1, total })}
        </span>
        <Button
          size="icon-sm"
          variant={repeat === "all" ? "secondary" : "ghost"}
          aria-label={t("playback.repeat")}
          aria-pressed={repeat === "all"}
          title={t(repeat === "all" ? "playback.repeatAll" : "playback.repeatOff")}
          onClick={toggleRepeat}
        >
          <Repeat aria-hidden="true" />
        </Button>
      </div>
    </div>
  )
}
