import { useTranslation } from "react-i18next"
import { Switch } from "@/components/ui/switch"
import { useUpdateSettings } from "@/hooks/useSettings"
import { SettingRow } from "./SettingRow"

/** 설정 > 플레이어: 자막 표시 — 플레이어의 CC 켜기/끄기와 같은 값(`subtitleVisible`) */
export function SubtitleVisibleRow({ value }: { value: boolean }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  return (
    <SettingRow label={t("settings.player.subtitleVisible")} description={t("settings.player.subtitleVisibleHint")}>
      <Switch aria-label={t("settings.player.subtitleVisible")} checked={value} onCheckedChange={(subtitleVisible) => update.mutate({ subtitleVisible })} />
    </SettingRow>
  )
}
