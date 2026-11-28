import { useTranslation } from "react-i18next"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Switch } from "@/components/ui/switch"
import { useModelReadiness } from "@/hooks/useAi"
import { SettingRow } from "../SettingRow"
import { SubtitleSettingsRows } from "./SubtitleSettingsRows"
import { LocalModelsList } from "./LocalModelsList"
import type { AiCtl } from "./types"

/** 로컬 자막(whisper)·의미 검색 설정과 로컬 모델 목록. 의미 검색은 e5 텍스트 모델이 준비된 뒤에만 켤 수 있다(CLIP 은 유사 미디어용). */
export function LocalSection({ ctl }: { ctl: AiCtl }) {
  const { t } = useTranslation()
  const { settings, patch } = ctl
  const { e5Ready } = useModelReadiness()
  return (
    <section aria-label={t("ai.local.section")} className="flex flex-col gap-3">
      <SettingRow label={t("ai.local.subtitle")} description={t("ai.local.subtitleHint")}>
        <div className="flex items-center gap-2">
          <Select
            value={settings.localStt.model}
            disabled={!settings.localStt.enabled}
            onValueChange={(model) => patch({ localStt: { model: model as "base" | "small" } })}
          >
            <SelectTrigger aria-label={t("ai.local.subtitleModel")} className="w-28">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="base">base</SelectItem>
              <SelectItem value="small">small</SelectItem>
            </SelectContent>
          </Select>
          <Switch aria-label={t("ai.local.subtitle")} checked={settings.localStt.enabled} onCheckedChange={(enabled) => patch({ localStt: { enabled } })} />
        </div>
      </SettingRow>
      <SubtitleSettingsRows ctl={ctl} />
      <SettingRow label={t("ai.local.semantic")} description={e5Ready ? t("ai.local.semanticHint") : t("ai.local.semanticNeedModels")}>
        <Switch
          aria-label={t("ai.local.semantic")}
          checked={settings.semanticSearch.enabled}
          disabled={!e5Ready}
          onCheckedChange={(enabled) => patch({ semanticSearch: { enabled } })}
        />
      </SettingRow>
      <LocalModelsList />
    </section>
  )
}
