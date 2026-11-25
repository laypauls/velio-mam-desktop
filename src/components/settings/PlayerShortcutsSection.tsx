import { useId } from "react"
import { useTranslation } from "react-i18next"
import { presetConflict } from "@/components/viewer/playRangeKeys"
import type { Settings } from "@/lib/ipc-contract"
import { formatShortcut } from "@/lib/shortcut"
import { CloseViewerShortcutRow } from "./CloseViewerShortcutRow"
import { HideShortcutRow } from "./HideShortcutRow"
import { PlayRangeKeysRow } from "./PlayRangeKeysRow"
import { SettingsSubsection } from "./SettingsSubsection"

/**
 * 설정 > 플레이어 > 단축키: 재생 구간 키 프리셋, 플레이어 패널 닫기·재생 정지 후 숨기기 단축키, 장면전환점 키 안내.
 * 구간 키 프리셋과 두 단축키가 겹치면 충돌 경고를 이 소제목 상단에 한 번만 보인다(겹친 키는 단축키가 먼저 처리된다).
 */
export function PlayerShortcutsSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const warnId = useId()
  const hideConflict = presetConflict(settings.playRangeKeys, settings.hideShortcut)
  const closeConflict = presetConflict(settings.playRangeKeys, settings.closeViewerShortcut)
  const conflicted = hideConflict !== null || closeConflict !== null
  return (
    <SettingsSubsection title={t("settings.sub.shortcuts")}>
      {conflicted && (
        <p id={warnId} role="status" className="rounded-lg border border-destructive/40 px-3 py-2 text-xs text-destructive break-keep">
          {[
            hideConflict !== null && t("settings.view.rangeKeysConflict", { key: formatShortcut(hideConflict) }),
            closeConflict !== null && t("settings.player.closeViewerRangeConflict", { key: formatShortcut(closeConflict) }),
          ]
            .filter(Boolean)
            .join(" ")}
        </p>
      )}
      <PlayRangeKeysRow value={settings.playRangeKeys} conflicted={conflicted} warnId={warnId} />
      <CloseViewerShortcutRow value={settings.closeViewerShortcut} hideValue={settings.hideShortcut} rangeKeys={settings.playRangeKeys} />
      <HideShortcutRow value={settings.hideShortcut} closeValue={settings.closeViewerShortcut} rangeKeys={settings.playRangeKeys} />
      <p className="text-xs text-muted-foreground break-keep">{t("settings.scene.keys")}</p>
    </SettingsSubsection>
  )
}
