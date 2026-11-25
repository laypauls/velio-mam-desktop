import { useTranslation } from "react-i18next"
import { Separator } from "@/components/ui/separator"
import { Switch } from "@/components/ui/switch"
import { useAppPaths, useUpdateSettings } from "@/hooks/useSettings"
import type { Settings } from "@/lib/ipc-contract"
import { BrandMark } from "./BrandMark"
import { LanguageSection } from "./LanguageSection"
import { SettingRow } from "./SettingRow"
import { SettingsSubsection } from "./SettingsSubsection"
import { ThemeSection } from "./ThemeSection"
import { UpdateSection } from "./UpdateSection"

function PathRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className="text-sm break-all">{value}</dd>
    </div>
  )
}

/** 설정 > 일반: 화면 테마 / 창 / 업데이트 / 앱 정보. (재스캔 간격은 "관리 폴더 > 스캔") */
export function GeneralTab({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const paths = useAppPaths()
  const rawVersion = import.meta.env.VITE_APP_VERSION as string | undefined
  const version = rawVersion ?? t("settings.general.devBuild")
  return (
    <div className="flex flex-col gap-4">
      <ThemeSection settings={settings} />
      <Separator />
      <LanguageSection settings={settings} />
      <Separator />
      <SettingsSubsection title={t("settings.sub.window")}>
        <SettingRow label={t("settings.general.closeToTray")} description={t("settings.general.closeToTrayHint")}>
          <Switch
            aria-label={t("settings.general.closeToTray")}
            checked={settings.closeToTray}
            onCheckedChange={(closeToTray) => update.mutate({ closeToTray })}
          />
        </SettingRow>
      </SettingsSubsection>
      <Separator />
      <UpdateSection />
      <Separator />
      <SettingsSubsection title={t("settings.general.about")}>
        <BrandMark version={rawVersion ? `v${rawVersion}` : version} />
        <dl className="flex flex-col gap-2">
          <PathRow label={t("settings.general.version")} value={version} />
          {paths.data && (
            <>
              <PathRow label={t("settings.general.dataDir")} value={paths.data.dataDir} />
              <PathRow label={t("settings.general.database")} value={paths.data.databasePath} />
              <PathRow label={t("settings.general.logs")} value={paths.data.logsDir} />
            </>
          )}
        </dl>
      </SettingsSubsection>
    </div>
  )
}
