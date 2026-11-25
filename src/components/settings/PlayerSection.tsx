import { useTranslation } from "react-i18next"
import { Separator } from "@/components/ui/separator"
import { Switch } from "@/components/ui/switch"
import { useUpdateSettings } from "@/hooks/useSettings"
import type { Settings } from "@/lib/ipc-contract"
import { OpenAutoplayRow } from "./OpenAutoplayRow"
import { PlayingStepRow } from "./PlayingStepRow"
import { PlayerShortcutsSection } from "./PlayerShortcutsSection"
import { SceneCutsSection } from "./SceneCutsSection"
import { SettingRow } from "./SettingRow"
import { SettingsSubsection } from "./SettingsSubsection"
import { SlideshowSecondsRow } from "./SlideshowSecondsRow"
import { SubtitleLanguageRow } from "./SubtitleLanguageRow"
import { SubtitleVisibleRow } from "./SubtitleVisibleRow"
import { ViewerPlacementRow } from "./ViewerPlacementRow"

/** 설정 > "플레이어" 페이지 본문(제목은 페이지 헤더): 재생 / 자막 / 장면전환점 / 컬렉션 연속 재생 / 단축키. */
export function PlayerSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  return (
    <section aria-label={t("settings.player.section")} className="flex flex-col gap-4">
      <SettingsSubsection title={t("settings.sub.playback")}>
        <SettingRow label={t("settings.view.videoBuiltIn")} description={t("settings.view.videoBuiltInHint")}>
          <Switch
            aria-label={t("settings.view.videoBuiltIn")}
            checked={settings.videoPlayerBuiltIn}
            onCheckedChange={(videoPlayerBuiltIn) => update.mutate({ videoPlayerBuiltIn })}
          />
        </SettingRow>
        <OpenAutoplayRow value={settings.mediaOpenAutoplay} />
        <PlayingStepRow value={settings.playerPlayingStepSec} />
        <ViewerPlacementRow value={settings.viewerPlacement} />
      </SettingsSubsection>
      <Separator />
      <SettingsSubsection title={t("settings.sub.subtitle")}>
        <SubtitleVisibleRow value={settings.subtitleVisible} />
        <SubtitleLanguageRow value={settings.subtitleDefaultLanguage} />
      </SettingsSubsection>
      <Separator />
      <SceneCutsSection settings={settings} />
      <Separator />
      <SettingsSubsection title={t("settings.sub.slideshow")}>
        <SlideshowSecondsRow value={settings.slideshowImageSeconds} />
      </SettingsSubsection>
      <Separator />
      <PlayerShortcutsSection settings={settings} />
    </section>
  )
}
