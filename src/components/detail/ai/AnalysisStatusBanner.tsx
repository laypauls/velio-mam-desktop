import { AlertCircle, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { aiErrorText, analysisKindKey } from "@/components/ai/labels"
import { Button } from "@/components/ui/button"
import { useAnalyzeActions } from "@/hooks/useAnalyze"
import type { AnalysisStatus } from "@/lib/ipc-contract-ai"

/** D6: 임베딩은 유사 탭이 안내하므로 요약에서 제외한다(실패 배너에만 포함). */
const inSummary = (s: AnalysisStatus) => s.kind !== "embedding"

/** 진행 중(대기·실행) 종류가 있으면 한 줄 배너 — "분석 중 · 완료/전체 — 설명 실행 중 · OCR 대기". */
export function AnalysisRunningBanner({ status }: { status: AnalysisStatus[] }) {
  const { t } = useTranslation()
  const tracked = status.filter((s) => inSummary(s) && ["queued", "running", "done", "failed"].includes(s.state))
  const active = tracked.filter((s) => s.state === "queued" || s.state === "running")
  if (active.length === 0) return null
  const done = tracked.filter((s) => s.state === "done").length
  const detail = active.map((s) => `${t(analysisKindKey(s.kind))} ${t(`ai.state.${s.state}`)}`).join(" · ")
  return (
    <div
      role="status"
      aria-busy="true"
      className="flex items-start gap-2 rounded-lg border border-border bg-muted/50 p-2.5 text-xs"
    >
      <Loader2 aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 animate-spin text-muted-foreground" />
      <p className="min-w-0 break-keep">
        <span className="font-medium">{t("ai.banner.running", { done, total: tracked.length })}</span>
        <span className="text-muted-foreground"> — {detail}</span>
      </p>
    </div>
  )
}

/** 실패한 종류가 있으면 사유와 [실패 항목 다시 시도] — 이 자산의 실패 단계만 다시 큐에 넣는다(`retryFailedAnalysis([assetId])`). */
export function AnalysisFailedBanner({
  assetId,
  status,
  busy,
}: {
  assetId: number
  status: AnalysisStatus[]
  busy: boolean
}) {
  const { t } = useTranslation()
  const { retryFailed } = useAnalyzeActions()
  const failed = status.filter((s) => s.state === "failed")
  if (failed.length === 0) return null
  const kinds = failed.map((s) => t(analysisKindKey(s.kind))).join(" · ")
  const reasons = [...new Set(failed.map((s) => aiErrorText(t, s.errorCode ?? "unknown")))]
  return (
    <div
      role="alert"
      className="flex flex-col items-start gap-2 rounded-lg border border-destructive/40 bg-destructive/5 p-2.5 text-xs"
    >
      <div className="flex items-start gap-2">
        <AlertCircle aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-destructive" />
        <div className="min-w-0 break-keep">
          <p className="font-medium text-destructive">{t("ai.banner.failed", { kinds })}</p>
          <p className="text-muted-foreground">{reasons.join(" / ")}</p>
        </div>
      </div>
      <Button size="xs" variant="outline" disabled={busy} onClick={() => void retryFailed([assetId])}>
        {t("ai.banner.retry")}
      </Button>
    </div>
  )
}
