import { useMutation, useQueryClient } from "@tanstack/react-query"
import { Pause, Play } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { qk } from "@/hooks/keys"
import { useFolderRemovalProgress } from "@/hooks/useFolderAdmin"
import { useJobSummary } from "@/hooks/useJobSummary"
import { useFolders } from "@/hooks/useLibrary"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { folderLabel } from "@/lib/folders"
import { formatNumber } from "@/lib/format"
import type { JobSummary } from "@/lib/ipc-contract"
import { JobQueuePopover } from "./JobQueuePopover"

function scanText(t: (k: string, o?: Record<string, unknown>) => string, scan: NonNullable<JobSummary["scan"]>, label: string): string {
  if (scan.total === null || scan.total === 0) {
    return t("status.scanIndeterminate", { path: label, processed: formatNumber(scan.processed) })
  }
  const percent = Math.min(100, Math.round((scan.processed / scan.total) * 100))
  return t("status.scan", { path: label, percent })
}

/** 하단 상태 표시줄: 스캔 진행·대기 작업·오류·전역 일시정지/재개. */
export function StatusBar({ onShowErrors }: { onShowErrors?: () => void }) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const { data } = useJobSummary()
  // 캐시된 폴더 목록만 사용 — 진행 이벤트(최대 4Hz)마다 추가 invoke 를 만들지 않는다
  const folders = useFolders().data
  const toggle = useMutation({
    mutationFn: (paused: boolean) => (paused ? api.resumeJobs(null) : api.pauseJobs(null)),
    onSuccess: (s) => qc.setQueryData(qk.jobs, s),
    onError: (e) => toast.error(t(errorKey(e))),
  })

  const removal = useFolderRemovalProgress()
  const removalPercent = removal && removal.total > 0 ? Math.min(100, Math.round((removal.done / removal.total) * 100)) : null

  const errors = data?.counts.scan.failed ?? 0
  const scan = data?.scan ?? null
  const scanFolder = scan ? folders?.find((f) => f.folderId === scan.folderId) : undefined
  const alias = scanFolder?.alias ?? null

  return (
    <footer role="status" aria-label={t("status.label")} className="flex items-center gap-3 border-t border-border px-3 py-1.5 text-xs text-muted-foreground">
      {data ? (
        <>
          {scan && (
            <span aria-live="polite" title={alias ? scanFolder?.path : undefined} className="min-w-0 truncate">
              {scanText(t, scan, scanFolder ? folderLabel(scanFolder) : scan.folderPath)}
            </span>
          )}
          {removal && (
            <span role="progressbar" aria-live="polite" aria-valuemin={0} aria-valuemax={100} aria-valuenow={removalPercent ?? undefined} className="min-w-0 truncate">
              {removalPercent === null ? t("status.removingIndeterminate") : t("status.removing", { percent: removalPercent })}
            </span>
          )}
          <JobQueuePopover summary={data} />
          {errors > 0 && (
            <span className="flex items-center gap-1 text-destructive">
              {t("status.errors", { count: formatNumber(errors) })}
              <Button size="xs" variant="ghost" onClick={onShowErrors} disabled={!onShowErrors}>
                {t("status.showErrors")}
              </Button>
            </span>
          )}
          {data.paused && <span>{t("status.paused")}</span>}
          <Button
            size="xs"
            variant="outline"
            className="ml-auto"
            disabled={toggle.isPending}
            onClick={() => toggle.mutate(data.paused)}
          >
            {data.paused ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}
            {t(data.paused ? "status.resume" : "status.pause")}
          </Button>
        </>
      ) : (
        <span>{t("common.loading")}</span>
      )}
    </footer>
  )
}
