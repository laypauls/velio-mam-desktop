import { useTranslation } from "react-i18next"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { WEEKDAYS } from "@/lib/rescan-policy"
import type { Weekday } from "@/lib/ipc-contract"
import { TimeField } from "./TimeField"
import type { TriggerFieldProps } from "./trigger-fields"

/** 요일·시각 트리거 옵션 — 요일을 하나도 고르지 않으면 매일. */
export function ScheduleFields({ draft, errors, onChange }: TriggerFieldProps) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">{t("settings.folders.detection.days")}</span>
        <ToggleGroup
          type="multiple"
          variant="outline"
          size="sm"
          spacing={1}
          aria-label={t("settings.folders.detection.days")}
          value={draft.days}
          onValueChange={(days) => onChange({ days: days as Weekday[] })}
          className="flex-wrap"
        >
          {WEEKDAYS.map((d) => (
            <ToggleGroupItem key={d} value={d}>
              {t(`watch.weekday.${d}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
        <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.detection.daysHint")}</p>
      </div>
      <TimeField label={t("settings.folders.detection.at")} value={draft.at} error={errors.at} onChange={(at) => onChange({ at })} />
    </div>
  )
}
