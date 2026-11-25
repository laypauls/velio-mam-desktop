import { useTranslation } from "react-i18next"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useUpdateSettings } from "@/hooks/useSettings"
import type { SubtitleLanguage } from "@/lib/ipc-contract"
import { SettingRow } from "./SettingRow"

const LANGUAGES: SubtitleLanguage[] = ["ko", "en", "ja", "zh"]
/** Radix SelectItem 은 빈 문자열 값을 쓸 수 없다 — "자막 끄기" 는 sentinel 로 렌더하고 저장할 때 null 로 바꾼다 */
const OFF = "__off__"

/** 설정 > 플레이어: 자막 기본 언어 — 여러 언어가 있으면 이 언어를 먼저 고른다. "자막 끄기" 는 자막 없이 시작(null) */
export function SubtitleLanguageRow({ value }: { value: SubtitleLanguage | null }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  return (
    <SettingRow label={t("settings.player.subtitleLanguage")} description={t("settings.player.subtitleLanguageHint")}>
      <Select
        value={value ?? OFF}
        onValueChange={(v) => update.mutate({ subtitleDefaultLanguage: v === OFF ? null : (v as SubtitleLanguage) })}
      >
        <SelectTrigger aria-label={t("settings.player.subtitleLanguage")} className="w-40">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={OFF}>{t("settings.player.subtitleLanguageOff")}</SelectItem>
          {LANGUAGES.map((code) => (
            <SelectItem key={code} value={code}>
              {t(`viewer.subtitle.lang.${code}`)}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </SettingRow>
  )
}
