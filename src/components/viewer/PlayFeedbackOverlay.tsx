import { ChevronLeft, ChevronRight, Pause, Play } from "lucide-react"
import { useTranslation } from "react-i18next"
import type { SeekFeedback, ToggleFeedback } from "./usePlayFeedback"

/** 화살표 웨이브 — 3개가 90ms 간격으로 순차 점등 */
const WAVE_DELAYS = ["", "[animation-delay:90ms]", "[animation-delay:180ms]"]

/** 스테이지 위에 잠깐 뜨는 재생·일시정지·앞으로·뒤로 표시 (부모는 `relative`). 시각 장식 전용 — 상태는 컨트롤 바 버튼 라벨이 전달한다 */
export function PlayFeedbackOverlay({ toggle, seek }: { toggle: ToggleFeedback | null; seek: SeekFeedback | null }) {
  const { t } = useTranslation()
  const base = "absolute top-1/2 grid -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full bg-background/60 text-foreground backdrop-blur-sm"
  const Chevron = seek?.dir === -1 ? ChevronLeft : ChevronRight
  return (
    <div aria-hidden="true" className="pointer-events-none absolute inset-0">
      {toggle !== null && (
        <div key={toggle.tick} data-slot="play-feedback-toggle" data-state={toggle.paused ? "paused" : "playing"} className={`${base} left-1/2 size-[72px] animate-feedback-pop motion-reduce:animate-feedback-fade`}>
          {toggle.paused ? <Pause className="size-8 fill-current stroke-0" /> : <Play className="size-8 fill-current stroke-0" />}
        </div>
      )}
      {seek !== null && (
        <div
          key={seek.dir}
          data-slot="play-feedback-seek"
          data-state={seek.dir > 0 ? "forward" : "back"}
          className={`${base} ${seek.dir > 0 ? "left-3/4" : "left-1/4"} size-[104px] content-center gap-0.5`}
        >
          <div key={seek.tick} className={`flex ${seek.dir > 0 ? "" : "flex-row-reverse"}`}>
            {WAVE_DELAYS.map((delay) => (
              <Chevron key={delay} className={`-mx-[3px] size-5 fill-current stroke-0 opacity-25 animate-feedback-wave motion-reduce:animate-none motion-reduce:opacity-80 ${delay}`} />
            ))}
          </div>
          <span className="text-[13px] font-semibold whitespace-nowrap tabular-nums">{t(seek.dir > 0 ? "viewer.seekForwardSec" : "viewer.seekBackSec", { sec: Math.round(seek.sec) })}</span>
        </div>
      )}
    </div>
  )
}
