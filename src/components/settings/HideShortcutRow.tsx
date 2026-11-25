import { useTranslation } from "react-i18next"
import type { PlayRangeKeys } from "@/lib/ipc-contract"
import { ShortcutRecorderRow } from "./ShortcutRecorderRow"

/** 설정 "재생 정지 후 숨기기 단축키" (기본 Esc) — 공용 단축키 기록 행. 패널 닫기 단축키와 같은 조합은 거부한다 */
export function HideShortcutRow({ value, closeValue, rangeKeys }: { value: string | null; closeValue: string | null; rangeKeys: PlayRangeKeys }) {
  const { t } = useTranslation()
  return (
    <ShortcutRecorderRow
      field="hideShortcut"
      label={t("settings.view.hideShortcut")}
      hint={t("settings.view.hideShortcutHint")}
      value={value}
      defaultValue="Escape"
      otherValue={closeValue}
      rangeKeys={rangeKeys}
    />
  )
}
