import { useTranslation } from "react-i18next"
import { INTERVAL_HOURS } from "@/lib/rescan-policy"
import { IntegerField } from "./IntegerField"
import type { TriggerFieldProps } from "./trigger-fields"

/** 간격(시간) 트리거 옵션. */
export function IntervalFields({ draft, errors, onChange }: TriggerFieldProps) {
  const { t } = useTranslation()
  return (
    <IntegerField
      label={t("settings.folders.detection.policyInterval")}
      unit={t("settings.folders.detection.hours")}
      value={draft.hours}
      min={INTERVAL_HOURS.min}
      max={INTERVAL_HOURS.max}
      error={errors.hours}
      onChange={(hours) => onChange({ hours })}
    />
  )
}
