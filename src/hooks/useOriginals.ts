import { useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { OriginalsResult } from "@/lib/ipc-contract"
import { summarizeFailures } from "@/lib/originals"
import { useSelectionStore } from "@/stores"
import { qk } from "./keys"
import { useProtectedAction } from "./useProtectedAction"

/** 원본 삭제(휴지통)·이동 실행 — 보호 흐름(D10) → command → 결과 toast·캐시 무효화. */
export function useOriginalsRun() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  const protect = useProtectedAction()

  const report = (kind: "delete" | "move", r: OriginalsResult) => {
    const counts = { count: formatNumber(r.succeeded), failed: formatNumber(r.failed.length) }
    if (r.failed.length > 0) {
      toast.warning(t(`originals.${kind}.partial`, counts), { description: summarizeFailures(t, r.failed) })
    }
    else toast.success(t(`originals.${kind}.done`, counts))
  }

  return async (kind: "delete" | "move", assetIds: number[], destDir?: string) => {
    try {
      const out = await protect(() =>
        kind === "delete" ? api.deleteOriginals(assetIds) : api.moveOriginals(assetIds, destDir ?? "")
      )
      if (!out.ran) return
      report(kind, out.value)
      useSelectionStore.getState().clear()
      for (const key of [qk.search, qk.assets, qk.folders, qk.jobs, qk.tags, qk.collections]) {
        void qc.invalidateQueries({ queryKey: key })
      }
    } catch (e) {
      toast.error(t(errorKey(e)))
    }
  }
}
