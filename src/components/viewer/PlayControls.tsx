import { Pause, Play, SkipBack, SkipForward, SquareChevronLeft, SquareChevronRight } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { formatDuration } from "@/lib/format"
import { useHoldRepeat } from "./useHoldRepeat"

/** 이전/다음 콘텐츠 이동 — 연속 재생은 대기열, 단독 보기는 갤러리의 현재 결과 순서. 처음/끝에서는 숨기지 않고 비활성화한다 */
export interface ContentNav {
  canPrev: boolean
  canNext: boolean
  onPrev: () => void
  onNext: () => void
}

interface Props {
  paused: boolean
  /** 프레임 이동 버튼 노출 (영상만) */
  frameStep: boolean
  /** 없으면 이전/다음 콘텐츠 버튼을 그리지 않는다 */
  nav?: ContentNav
  /** 프레임 이동 버튼 비활성 (시작 전) */
  stepDisabled?: boolean
  onTogglePlay: () => void
  onStep: (dir: 1 | -1) => void | Promise<unknown>
}

/** 프레임 이동 버튼 — 누르고 있으면 반복한다 (`useHoldRepeat`) */
function FrameStepButton({ dir, label, title, disabled, onStep }: { dir: 1 | -1; label: string; title: string; disabled?: boolean; onStep: (dir: 1 | -1) => void | Promise<unknown> }) {
  const hold = useHoldRepeat(() => onStep(dir))
  return (
    <Button size="icon-sm" variant="ghost" aria-label={label} title={title} disabled={disabled} {...hold}>
      {dir < 0 ? <SquareChevronLeft aria-hidden="true" /> : <SquareChevronRight aria-hidden="true" />}
    </Button>
  )
}

/** 재생 그룹 — `[이전 콘텐츠] [이전 프레임] (재생) [다음 프레임] [다음 콘텐츠]` */
export function PlayControls({ paused, frameStep, nav, stepDisabled, onTogglePlay, onStep }: Props) {
  const { t } = useTranslation()
  const tip = (label: string, key: string) => t("viewer.withKey", { label, key })
  return (
    <div className="flex items-center gap-1">
      {nav && (
        <Button size="icon-sm" variant="ghost" aria-label={t("viewer.contentPrev")} title={tip(t("viewer.contentPrev"), "Shift+P")} disabled={!nav.canPrev} onClick={nav.onPrev}>
          <SkipBack aria-hidden="true" />
        </Button>
      )}
      {frameStep && (
        <FrameStepButton dir={-1} label={t("viewer.framePrev")} title={tip(t("viewer.framePrev"), ",")} disabled={stepDisabled} onStep={onStep} />
      )}
      <Button
        size="icon"
        variant="default"
        className="rounded-full"
        aria-label={paused ? t("viewer.play") : t("viewer.pause")}
        title={tip(paused ? t("viewer.play") : t("viewer.pause"), "Space")}
        onClick={onTogglePlay}
      >
        {paused ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}
      </Button>
      {frameStep && (
        <FrameStepButton dir={1} label={t("viewer.frameNext")} title={tip(t("viewer.frameNext"), ".")} disabled={stepDisabled} onStep={onStep} />
      )}
      {nav && (
        <Button size="icon-sm" variant="ghost" aria-label={t("viewer.contentNext")} title={tip(t("viewer.contentNext"), "Shift+N")} disabled={!nav.canNext} onClick={nav.onNext}>
          <SkipForward aria-hidden="true" />
        </Button>
      )}
    </div>
  )
}

/** `현재 / 전체` 시간 — 재생 그룹 오른쪽 */
export function PlayTime({ currentSec, totalSec }: { currentSec: number; totalSec: number }) {
  const { t } = useTranslation()
  const current = formatDuration(currentSec * 1000)
  const total = formatDuration(totalSec * 1000)
  return (
    <span className="text-xs text-muted-foreground tabular-nums" aria-label={t("viewer.position", { current, total })}>
      {current} / {total}
    </span>
  )
}
