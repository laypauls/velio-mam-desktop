import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group"
import { useAiUsage } from "@/hooks/useAi"
import { formatNumber } from "@/lib/format"

const PERIODS = [7, 30] as const

/** D7: 사용량 — 기간(최근 7·30일)별 일·공급자·모델 집계표. 비용 환산·한도는 두지 않는다. */
export function UsageSection() {
  const { t } = useTranslation()
  const [days, setDays] = useState<number>(7)
  const usage = useAiUsage(days)
  const rows = usage.data ?? []
  return (
    <section aria-label={t("ai.usage.section")} className="flex flex-col gap-2">
      <div className="flex items-center gap-2">
        <h3 className="text-sm font-medium">{t("ai.usage.title")}</h3>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          className="ml-auto"
          value={String(days)}
          aria-label={t("ai.usage.period")}
          onValueChange={(v) => v && setDays(Number(v))}
        >
          {PERIODS.map((p) => (
            <ToggleGroupItem key={p} value={String(p)}>
              {t("ai.usage.lastDays", { count: p })}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      {usage.isError ? (
        <p role="alert" className="text-xs text-destructive">
          {t("ai.usage.error")}
        </p>
      ) : rows.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t("ai.usage.empty")}</p>
      ) : (
        <div className="max-h-56 overflow-y-auto rounded-lg border border-border">
          <table className="w-full text-xs">
            <caption className="sr-only">{t("ai.usage.title")}</caption>
            <thead className="sticky top-0 bg-muted text-left text-muted-foreground">
              <tr>
                {(["day", "provider", "model", "requests", "failures", "tokens", "images", "auth"] as const).map((c) => (
                  <th key={c} scope="col" className="px-2 py-1.5 font-medium">
                    {t(`ai.usage.col.${c}`)}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody className="divide-y divide-border">
              {rows.map((r) => (
                <tr key={`${r.day}|${r.provider}|${r.model}|${r.authMode ?? "apiKey"}`}>
                  <td className="px-2 py-1 tabular-nums">{r.day}</td>
                  <td className="px-2 py-1">{t(`ai.provider.name.${r.provider}`)}</td>
                  <td className="px-2 py-1">{r.model}</td>
                  <td className="px-2 py-1 tabular-nums">{formatNumber(r.requests)}</td>
                  <td className="px-2 py-1 tabular-nums">{formatNumber(r.failures)}</td>
                  <td className="px-2 py-1 tabular-nums">{formatNumber(r.inputTokens + r.outputTokens)}</td>
                  <td className="px-2 py-1 tabular-nums">{formatNumber(r.images)}</td>
                  <td className="px-2 py-1">{t(`ai.usage.authMode.${r.authMode ?? "apiKey"}`)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  )
}
