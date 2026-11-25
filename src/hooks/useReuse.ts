import { useMutation } from "@tanstack/react-query"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { ConflictPolicy, ExportProgress } from "@/lib/ipc-contract"
import { formatNumber } from "@/lib/format"

/** 클립보드 복사 (Windows 파일 클립보드) + 결과 toast. */
export function useCopyToClipboard() {
  const api = useApi()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: (assetIds: number[]) => api.copyToClipboard(assetIds),
    onSuccess: (_v, ids) => void toast.success(t("reuse.copied", { count: formatNumber(ids.length) })),
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 원본 열기 / 탐색기에서 위치 보기 */
export function useOriginalActions() {
  const api = useApi()
  const { t } = useTranslation()
  const onError = (e: unknown) => void toast.error(t(errorKey(e)))
  const open = useMutation({ mutationFn: (id: number) => api.openOriginal(id), onError })
  const reveal = useMutation({ mutationFn: (id: number) => api.revealInExplorer(id), onError })
  return { open: (id: number) => open.mutate(id), reveal: (id: number) => reveal.mutate(id) }
}

export type ExportPhase = "idle" | "running" | "done"

export interface ExportOptions {
  conflict: ConflictPolicy
  includeManifest: boolean
}

/**
 * 내보내기 실행: 진행률 이벤트를 먼저 구독한 뒤 command 를 호출한다 (이벤트가 응답보다 먼저 올 수 있어
 * taskId 별로 모아 두고 응답의 taskId 로 고른다). 완료(finished) 이벤트에서 toast 를 띄운다.
 */
export function useExportRun() {
  const api = useApi()
  const { t } = useTranslation()
  const [taskId, setTaskId] = useState<string | null>(null)
  const [events, setEvents] = useState<Record<string, ExportProgress>>({})
  const [phase, setPhase] = useState<ExportPhase>("idle")
  const [error, setError] = useState<unknown>(null)
  const unlisten = useRef<(() => void) | null>(null)
  const notified = useRef<Set<string>>(new Set())

  const stop = () => {
    unlisten.current?.()
    unlisten.current = null
  }
  useEffect(() => stop, [])

  const progress = taskId !== null ? (events[taskId] ?? null) : null

  useEffect(() => {
    if (!progress?.finished || notified.current.has(progress.taskId)) return
    notified.current.add(progress.taskId)
    stop()
    setPhase("done")
    const counts = { done: formatNumber(progress.done), failed: formatNumber(progress.failed), skipped: formatNumber(progress.skipped) }
    if (progress.failed > 0) toast.warning(t("export.finishedWithFailures", counts))
    else toast.success(t("export.finished", counts))
  }, [progress, t])

  const start = async (assetIds: number[], destDir: string, opts: ExportOptions) => {
    setError(null)
    setPhase("running")
    try {
      unlisten.current = await api.onExportProgress((p) => setEvents((prev) => ({ ...prev, [p.taskId]: p })))
      const started = await api.exportAssets({ assetIds, destDir, conflict: opts.conflict, includeManifest: opts.includeManifest })
      setTaskId(started.taskId)
    } catch (e) {
      stop()
      setError(e)
      setPhase("idle")
    }
  }

  const reset = () => {
    stop()
    setTaskId(null)
    setEvents({})
    setError(null)
    setPhase("idle")
  }

  return { phase, progress, error, start, reset }
}

export type ExportRun = ReturnType<typeof useExportRun>
