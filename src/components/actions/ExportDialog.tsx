import { FolderOpen } from "lucide-react"
import { useId, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Label } from "@/components/ui/label"
import { Progress } from "@/components/ui/progress"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useExportRun, type ExportRun } from "@/hooks/useReuse"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { ConflictPolicy, ExportProgress } from "@/lib/ipc-contract"
import { useActionsStore } from "@/stores"

const POLICIES: ConflictPolicy[] = ["skip", "rename", "overwrite"]

function ProgressView({ progress, total }: { progress: ExportProgress | null; total: number }) {
  const { t } = useTranslation()
  const done = progress?.done ?? 0
  const all = progress?.total ?? total
  const percent = all > 0 ? Math.round((done / all) * 100) : 0
  return (
    <div className="flex flex-col gap-2" aria-live="polite">
      <Progress value={percent} aria-label={t("export.progress")} />
      <p className="text-sm tabular-nums">{t("export.counts", { done: formatNumber(done), total: formatNumber(all) })}</p>
      <p className="text-xs text-muted-foreground tabular-nums">
        {t("export.failedSkipped", { failed: formatNumber(progress?.failed ?? 0), skipped: formatNumber(progress?.skipped ?? 0) })}
      </p>
    </div>
  )
}

function ExportBody({ assetIds, run, onClose }: { assetIds: number[]; run: ExportRun; onClose: () => void }) {
  const { t } = useTranslation()
  const api = useApi()
  const policyId = useId()
  const manifestId = useId()
  const [destDir, setDestDir] = useState<string | null>(null)
  const [conflict, setConflict] = useState<ConflictPolicy>("rename")
  const [manifest, setManifest] = useState(true)
  const [pickError, setPickError] = useState<string | null>(null)

  const pick = async () => {
    setPickError(null)
    try {
      const dir = await api.pickDirectory()
      if (dir !== null) setDestDir(dir) // 취소(null)는 아무것도 바꾸지 않는다
    } catch (e) {
      setPickError(t(errorKey(e)))
    }
  }

  const running = run.phase === "running"
  const finished = run.phase === "done"
  const failedAny = (run.progress?.failed ?? 0) > 0

  return (
    <>
      <DialogHeader>
        <DialogTitle>{finished ? t("export.doneTitle") : t("export.title")}</DialogTitle>
        <DialogDescription>{t("export.description", { count: formatNumber(assetIds.length) })}</DialogDescription>
      </DialogHeader>
      {run.phase === "idle" ? (
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-1.5">
            <span className="text-sm font-medium">{t("export.destination")}</span>
            <div className="flex items-center gap-2">
              <Button variant="outline" onClick={() => void pick()}>
                <FolderOpen aria-hidden="true" />
                {t("export.pick")}
              </Button>
              <span className="min-w-0 flex-1 truncate text-sm text-muted-foreground" title={destDir ?? undefined}>
                {destDir ?? t("export.noDestination")}
              </span>
            </div>
            {pickError !== null && (
              <p role="alert" className="text-xs text-destructive">
                {pickError}
              </p>
            )}
          </div>
          <div className="flex flex-col gap-1.5">
            <Label htmlFor={policyId} className="!items-start">
              {t("export.conflict")}
            </Label>
            <Select value={conflict} onValueChange={(v) => setConflict(v as ConflictPolicy)}>
              <SelectTrigger id={policyId} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {POLICIES.map((p) => (
                  <SelectItem key={p} value={p}>
                    {t(`export.policy.${p}`)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-2">
            <Checkbox id={manifestId} checked={manifest} onCheckedChange={(v) => setManifest(v === true)} />
            <Label htmlFor={manifestId}>{t("export.manifest")}</Label>
          </div>
          {run.error !== null && (
            <p role="alert" className="text-xs break-keep text-destructive">
              {t(errorKey(run.error))}
            </p>
          )}
        </div>
      ) : (
        <div className="flex flex-col gap-3">
          <ProgressView progress={run.progress} total={assetIds.length} />
          {finished && failedAny && (
            <p role="alert" className="text-sm break-keep text-destructive">
              {t("export.failedNotice", { count: formatNumber(run.progress?.failed ?? 0) })}
            </p>
          )}
        </div>
      )}
      <DialogFooter>
        {finished ? (
          <Button onClick={onClose}>{t("common.close")}</Button>
        ) : (
          <>
            <Button variant="outline" disabled={running} onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={running || destDir === null}
              onClick={() => destDir !== null && void run.start(assetIds, destDir, { conflict, includeManifest: manifest })}
            >
              {t("export.start")}
            </Button>
          </>
        )}
      </DialogFooter>
    </>
  )
}

/** 내보내기 — 폴더 선택 → 충돌 정책·매니페스트 → 진행률(`export://progress`) → 완료. */
export function ExportDialog() {
  const ids = useActionsStore((s) => s.exportIds)
  const closeStore = useActionsStore((s) => s.closeExport)
  const run = useExportRun()
  // 실행 중에는 닫을 수 없다 (취소 command 가 계약에 없다)
  const close = () => {
    if (run.phase === "running") return
    run.reset()
    closeStore()
  }
  return (
    <Dialog open={ids !== null} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-lg" showCloseButton={false}>
        {ids !== null && <ExportBody assetIds={ids} run={run} onClose={close} />}
      </DialogContent>
    </Dialog>
  )
}
