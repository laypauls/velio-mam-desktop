import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { activeHours, scheduleSegments } from "./scheduleHours"

const TICKS = [0, 3, 6, 9, 12, 15, 18, 21, 24]
const pad = (h: number) => String(h).padStart(2, "0")

/**
 * 자동 분석 예약 시간의 24시간 막대 — 활성 시간대를 칠한다(자정을 넘기면 양 끝, 시작 = 종료면 전체).
 * 막대는 장식(aria-hidden)이고, 같은 내용을 한 줄 문장으로도 보여 스크린리더가 읽는다.
 */
export function ScheduleTimeline({ startHour, endHour }: { startHour: number; endHour: number }) {
  const { t } = useTranslation()
  const { kind } = scheduleSegments(startHour, endHour)
  const active = activeHours(startHour, endHour)
  const params = { start: pad(startHour), end: pad(endHour) }
  const status = kind === "allDay" ? t("ai.auto.timelineAllDay") : t(kind === "wrap" ? "ai.auto.timelineWrap" : "ai.auto.timelineWindow", params)
  return (
    <div className="flex flex-col gap-1.5">
      <div aria-hidden="true" className="flex flex-col gap-1">
        <div className="grid grid-cols-24 gap-px overflow-hidden rounded-sm">
          {active.map((on, h) => (
            <div key={h} className={cn("h-3", on ? "bg-primary" : "bg-muted")} />
          ))}
        </div>
        <div className="flex justify-between text-[10px] leading-none text-muted-foreground tabular-nums">
          {TICKS.map((h) => (
            <span key={h}>{h}</span>
          ))}
        </div>
      </div>
      <p role="status" className="text-xs text-muted-foreground break-keep">
        {status}
      </p>
    </div>
  )
}
