import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useProviders } from "@/hooks/useAi"
import type { AiProvider } from "@/lib/ipc-contract-ai"
import { ProviderRow } from "./ProviderRow"
import type { AiCtl } from "./types"

/** Radix Select 는 빈 문자열 value 를 허용하지 않아 "선택 안 함" 은 sentinel 로 렌더하고 저장 시 null 로 바꾼다. */
const NONE = "__none__"
const PROVIDERS: AiProvider[] = ["openai", "gemini", "claude"]

/** D2: 기본 공급자 선택 + 공급자별 쓰기 전용 API 키. */
export function ProvidersSection({ ctl }: { ctl: AiCtl }) {
  const { t } = useTranslation()
  const providers = useProviders()
  const [persist, setPersist] = useState(true)
  const byName = new Map((providers.data ?? []).map((p) => [p.provider, p]))
  return (
    <section aria-label={t("ai.provider.section")} className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{t("ai.provider.default")}</span>
        <Select
          value={ctl.settings.defaultProvider ?? NONE}
          onValueChange={(v) => ctl.patch({ defaultProvider: v === NONE ? null : (v as AiProvider) })}
        >
          <SelectTrigger aria-label={t("ai.provider.default")} className="w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value={NONE}>{t("ai.provider.none")}</SelectItem>
            {PROVIDERS.map((p) => (
              <SelectItem key={p} value={p}>
                {t(`ai.provider.name.${p}`)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      <Label className="!items-center gap-2 font-normal">
        <Checkbox checked={persist} onCheckedChange={(v) => setPersist(v === true)} />
        {t("ai.provider.persist")}
      </Label>
      <p className="text-xs break-keep text-muted-foreground">{t("ai.provider.writeOnly")}</p>
      <ul aria-label={t("ai.provider.list")} className="flex flex-col gap-2">
        {PROVIDERS.map((p) => {
          const status = byName.get(p)
          return status ? <ProviderRow key={p} status={status} persist={persist} /> : null
        })}
      </ul>
    </section>
  )
}
