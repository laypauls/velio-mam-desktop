import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { AI_LIMITS } from "@/lib/ipc-contract-ai"
import { NumberField } from "../NumberField"
import { SettingRow } from "../SettingRow"
import { ScheduleTimeline } from "./ScheduleTimeline"
import type { AiCtl } from "./types"

/** D1: 자동·백그라운드·예약 분석. 동의 전에는 자동·백그라운드 스위치를 비활성화한다. */
export function AutomationSection({ ctl }: { ctl: AiCtl }) {
  const { t } = useTranslation()
  const { settings, patch, commit } = ctl
  const consented = settings.consent.given
  const { startHour, endHour, enabled } = settings.schedule
  return (
    <section aria-label={t("ai.auto.section")} className="flex flex-col gap-3">
      <SettingRow label={t("ai.auto.new")} description={`${consented ? t("ai.auto.newHint") : t("ai.auto.needConsent")} ${t("ai.auto.newSubtitleHint")}`}>
        <Switch aria-label={t("ai.auto.new")} checked={settings.autoAnalyzeNew} disabled={!consented} onCheckedChange={(autoAnalyzeNew) => patch({ autoAnalyzeNew })} />
      </SettingRow>
      <SettingRow label={t("ai.auto.background")} description={consented ? t("ai.auto.backgroundHint") : t("ai.auto.needConsent")}>
        <Switch aria-label={t("ai.auto.background")} checked={settings.backgroundAnalyze} disabled={!consented} onCheckedChange={(backgroundAnalyze) => patch({ backgroundAnalyze })} />
      </SettingRow>
      <SettingRow label={t("ai.auto.schedule")} description={t("ai.auto.scheduleHint")}>
        <Switch aria-label={t("ai.auto.schedule")} checked={enabled} onCheckedChange={(on) => patch({ schedule: { enabled: on } })} />
      </SettingRow>
      {enabled && (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap gap-6">
            <NumberField
              label={t("ai.auto.startHour")}
              unit={t("settings.unit.hour")}
              value={startHour}
              min={AI_LIMITS.scheduleHour.min}
              max={AI_LIMITS.scheduleHour.max}
              onCommit={(v) => commit({ schedule: { startHour: v } })}
            />
            <NumberField
              label={t("ai.auto.endHour")}
              unit={t("settings.unit.hour")}
              value={endHour}
              min={AI_LIMITS.scheduleHour.min}
              max={AI_LIMITS.scheduleHour.max}
              onCommit={(v) => commit({ schedule: { endHour: v } })}
            />
          </div>
          <div className="max-w-xl">
            <ScheduleTimeline startHour={startHour} endHour={endHour} />
          </div>
        </div>
      )}
    </section>
  )
}
