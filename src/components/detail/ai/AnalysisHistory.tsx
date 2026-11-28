import { ChevronRight } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { aiErrorText, analysisKindKey } from "@/components/ai/labels"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { useAnalysisHistory } from "@/hooks/useAi"
import { formatDateTime } from "@/lib/format"

/** D4: 분석 이력 — 파일별 성공/실패 기록. 펼칠 때만 조회한다. */
export function AnalysisHistory({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const history = useAnalysisHistory(assetId, open)
  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <CollapsibleTrigger className="flex items-center gap-1 text-xs font-medium text-muted-foreground hover:text-foreground">
        <ChevronRight aria-hidden="true" className={open ? "size-3.5 rotate-90" : "size-3.5"} />
        {t("ai.history.title")}
      </CollapsibleTrigger>
      <CollapsibleContent>
        {history.isError ? (
          <p role="alert" className="pt-1 text-xs text-destructive">
            {t("ai.history.error")}
          </p>
        ) : (history.data?.length ?? 0) === 0 ? (
          <p className="pt-1 text-xs text-muted-foreground">{t("ai.history.empty")}</p>
        ) : (
          <ul aria-label={t("ai.history.list")} className="flex flex-col gap-1 pt-1 text-xs">
            {history.data?.map((r) => (
              <li key={r.runId} className="flex flex-wrap gap-x-2 text-muted-foreground">
                <span className="tabular-nums">{formatDateTime(r.at)}</span>
                <span>{t(analysisKindKey(r.kind))}</span>
                <span>{r.provider}</span>
                <span className={r.state === "failed" ? "text-destructive" : undefined}>
                  {r.state === "failed" ? `${t("ai.state.failed")} (${aiErrorText(t, r.errorCode ?? "unknown")})` : t("ai.state.done")}
                </span>
              </li>
            ))}
          </ul>
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}
