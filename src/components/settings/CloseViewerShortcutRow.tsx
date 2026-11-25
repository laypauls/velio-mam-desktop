import { useTranslation } from "react-i18next"
import type { PlayRangeKeys } from "@/lib/ipc-contract"
import { ShortcutRecorderRow } from "./ShortcutRecorderRow"

/** 설정 "플레이어 패널 닫기" 단축키 (기본 Q) — 공용 단축키 기록 행. 숨기기 단축키와 같은 조합은 거부한다 */
export function CloseViewerShortcutRow({ value, hideValue, rangeKeys }: { value: string | null; hideValue: string | null; rangeKeys: PlayRangeKeys }) {
  const { t } = useTranslation()
  return (
    <ShortcutRecorderRow
      field="closeViewerShortcut"
      label={t("settings.player.closeViewer")}
      hint={t("settings.player.closeViewerHint")}
      value={value}
      defaultValue="Q"
      otherValue={hideValue}
      rangeKeys={rangeKeys}
    />
  )
}
