import { RefreshCw, Sparkles } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useAnalyzeActions } from "@/hooks/useAnalyze"

/** D5: 설정에서 미분석 전체 분석 / 실패 재시도. 동의 전이면 동의 Dialog 를 거친다. */
export function BatchSection() {
  const { t } = useTranslation()
  const { analyzeAll, retryFailed } = useAnalyzeActions()
  const [busy, setBusy] = useState(false)
  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true)
    await fn()
    setBusy(false)
  }
  return (
    <section aria-label={t("ai.batch.section")} className="flex flex-col gap-1.5">
      <div className="flex flex-wrap gap-2">
        <Button size="sm" variant="outline" disabled={busy} onClick={() => void run(() => analyzeAll(true))}>
          <Sparkles aria-hidden="true" />
          {t("ai.batch.all")}
        </Button>
        <Button size="sm" variant="outline" disabled={busy} onClick={() => void run(() => retryFailed(null))}>
          <RefreshCw aria-hidden="true" />
          {t("ai.batch.retry")}
        </Button>
      </div>
      <p className="text-xs break-keep text-muted-foreground">{t("ai.batch.hint")}</p>
    </section>
  )
}
