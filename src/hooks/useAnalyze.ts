import { useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { AnalysisKind, QueueResult } from "@/lib/ipc-contract-ai"
import { qk } from "./keys"
import { useConsentFlow } from "./useConsent"

/** 분석 요청(개별·일괄·전체·재시도). 동의 전이면 동의 Dialog 를 거쳐 한 번 재시도하고, 결과는 toast 로 알린다. */
export function useAnalyzeActions() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  const { withConsent } = useConsentFlow()

  const run = async (command: () => Promise<QueueResult>): Promise<QueueResult | null> => {
    try {
      const res = await withConsent(command)
      if (!res.ran) return null
      const { queued, skipped } = res.value
      const counts = { queued: formatNumber(queued), skipped: formatNumber(skipped) }
      if (queued > 0) toast.success(t("ai.queued", counts), skipped > 0 ? { description: t("ai.skippedHint") } : undefined)
      else toast.info(t("ai.nothingQueued"), { description: t("ai.skippedHint") })
      void qc.invalidateQueries({ queryKey: qk.assetAiAll })
      return res.value
    } catch (e) {
      toast.error(t(errorKey(e)))
      return null
    }
  }

  return {
    analyze: (assetIds: number[], kinds: AnalysisKind[] | null, force: boolean) =>
      run(() => api.analyzeAssets(assetIds, kinds, force)),
    analyzeAll: (onlyUnanalyzed: boolean) => run(() => api.analyzeAll(onlyUnanalyzed)),
    retryFailed: (assetIds: number[] | null) => run(() => api.retryFailedAnalysis(assetIds)),
  }
}
