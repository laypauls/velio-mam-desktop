import { Sparkles } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useAssetAi } from "@/hooks/useAi"
import { formatDateTime } from "@/lib/format"
import type { MediaType } from "@/lib/ipc-contract"
import { AnalysisActions } from "./AnalysisActions"
import { AnalysisHistory } from "./AnalysisHistory"
import { AnalysisResult } from "./AnalysisResult"
import { AnalysisFailedBanner, AnalysisRunningBanner } from "./AnalysisStatusBanner"
import { TranscriptSection } from "./TranscriptSection"

/** D4: 상세 패널 AI 탭 — 결과(편집 가능)·종류별 상태·분석 버튼·이력·STT 원문. */
export function AiTab({ assetId, mediaType }: { assetId: number; mediaType: MediaType }) {
  const { t } = useTranslation()
  const { data, isPending, isError, refetch } = useAssetAi(assetId)

  if (isPending) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="h-40 w-full" />
  }
  if (isError || !data) {
    return (
      <div role="alert" className="flex flex-col items-start gap-2 text-sm text-destructive">
        <span className="break-keep">{t("ai.loadError")}</span>
        <Button size="sm" variant="outline" onClick={() => void refetch()}>
          {t("common.retry")}
        </Button>
      </div>
    )
  }
  const busy = data.status.some((s) => s.state === "queued" || s.state === "running")
  const failed = data.status.some((s) => s.state === "failed")
  const analysis = data.analysis

  return (
    <div className="flex flex-col gap-4">
      <AnalysisRunningBanner status={data.status} />
      <AnalysisFailedBanner assetId={assetId} status={data.status} busy={busy} />
      {analysis ? (
        <AnalysisResult assetId={assetId} analysis={analysis} />
      ) : (
        <div className="flex flex-col items-center gap-1.5 rounded-lg border border-dashed border-border p-4 text-center">
          <Sparkles aria-hidden="true" className="size-6 text-muted-foreground" />
          <p className="text-sm font-medium break-keep">{t("ai.empty.title")}</p>
          <p className="text-xs break-keep text-muted-foreground">{t("ai.empty.description")}</p>
          <AnalysisActions assetId={assetId} hasResult={false} edited={false} busy={busy} />
        </div>
      )}
      {mediaType !== "image" && <TranscriptSection assetId={assetId} transcript={data.transcript} />}
      {analysis && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          {!busy && !failed && (
            <p className="min-w-0 text-xs break-words text-muted-foreground">
              {t("ai.result.meta", {
                provider: analysis.provider,
                model: analysis.model,
                at: formatDateTime(analysis.analyzedAt),
              })}
            </p>
          )}
          <AnalysisActions assetId={assetId} hasResult edited={analysis.edited} busy={busy} />
        </div>
      )}
      {analysis?.edited && <p className="text-xs break-keep text-muted-foreground">{t("ai.overwriteWarning")}</p>}
      <AnalysisHistory assetId={assetId} />
    </div>
  )
}
