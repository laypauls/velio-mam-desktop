import { useMutation, useQueryClient } from "@tanstack/react-query"
import { ListTodo, Loader2, Pause, Play } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { JobKind, JobSummary } from "@/lib/ipc-contract"

/** 표시 대상 유형. `scan`(진행률 별도 표시)·`index`(실행기 없음, 항상 0)는 제외한다. */
const QUEUE_KINDS = ["meta", "thumb", "hash", "ai", "scene", "plugin"] as const satisfies readonly JobKind[]
type QueueKind = (typeof QUEUE_KINDS)[number]

/** 하단 상태 표시줄의 백그라운드 대기열 요약 칩 + 상세 Popover. 작업이 없어도 "백그라운드 작업" 버튼은 항상 보이고, 열면 빈 상태 문구를 보인다. */
export function JobQueuePopover({ summary }: { summary: JobSummary }) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const toggleKind = useMutation({
    mutationFn: ({ kind, paused }: { kind: QueueKind; paused: boolean }) =>
      paused ? api.resumeJobs(kind) : api.pauseJobs(kind),
    onSuccess: (s) => qc.setQueryData(qk.jobs, s),
    onError: (e) => toast.error(t(errorKey(e))),
  })

  const waiting = (kind: QueueKind) => summary.counts[kind].pending + summary.counts[kind].running
  const active = QUEUE_KINDS.filter((kind) => waiting(kind) > 0 || summary.pausedKinds.includes(kind))

  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button size="xs" variant="ghost" className="gap-2">
          {active.length === 0 && (
            <span className="flex items-center gap-1">
              <ListTodo aria-hidden="true" />
              {t("status.detail.title")}
            </span>
          )}
          {active.map((kind) => (
            <span key={kind} className="flex items-center gap-1">
              {summary.counts[kind].running > 0 && <Loader2 aria-hidden="true" className="animate-spin" />}
              {summary.pausedKinds.includes(kind) && <Pause aria-hidden="true" />}
              {t(`status.kind.${kind}`)} {formatNumber(waiting(kind))}
            </span>
          ))}
        </Button>
      </PopoverTrigger>
      <PopoverContent side="top" align="start" className="w-96 gap-3 rounded-lg">
        <h2 className="font-medium">{t("status.detail.title")}</h2>
        <table className="w-full text-left">
          <thead className="text-xs text-muted-foreground">
            <tr>
              <th className="py-1 font-normal">{t("status.detail.kind")}</th>
              <th className="py-1 text-right font-normal">{t("status.detail.pending")}</th>
              <th className="py-1 text-right font-normal">{t("status.detail.running")}</th>
              <th className="py-1 text-right font-normal">{t("status.detail.failed")}</th>
              <th className="w-8">
                <span className="sr-only">{t("status.detail.action")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {QUEUE_KINDS.map((kind) => {
              const c = summary.counts[kind]
              const paused = summary.pausedKinds.includes(kind)
              const name = t(`status.kind.${kind}`)
              return (
                <tr key={kind} className="border-t border-border">
                  <th scope="row" className="py-1 font-normal break-keep">{name}</th>
                  <td className="py-1 text-right tabular-nums">{formatNumber(c.pending)}</td>
                  <td className="py-1 text-right tabular-nums">{formatNumber(c.running)}</td>
                  <td className="py-1 text-right tabular-nums">{formatNumber(c.failed)}</td>
                  <td className="py-1 text-right">
                    <Button
                      size="icon-xs"
                      variant="ghost"
                      disabled={toggleKind.isPending}
                      aria-label={t(paused ? "status.detail.resumeKind" : "status.detail.pauseKind", { name })}
                      onClick={() => toggleKind.mutate({ kind, paused })}
                    >
                      {paused ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}
                    </Button>
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
        {active.length === 0 && <p className="text-sm break-keep text-muted-foreground">{t("status.detail.empty")}</p>}
      </PopoverContent>
    </Popover>
  )
}
