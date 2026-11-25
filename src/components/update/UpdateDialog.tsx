import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Progress } from "@/components/ui/progress"
import { useUpdater } from "@/hooks/useUpdater"
import { currentLocale } from "@/i18n/applyLanguage"
import { formatBytes } from "@/lib/format"
import { updateErrorKey } from "@/lib/update-errors"
import { useUpdateStore } from "@/stores/update"

function formatReleaseDate(date: string | null): string | null {
  if (!date) return null
  const d = new Date(date)
  return Number.isNaN(d.getTime()) ? date : d.toLocaleDateString(currentLocale())
}

/** 업데이트 승인 Dialog — 현재→새 버전·릴리스 노트, 나중에 / 지금 설치, 진행률, 오류+재시도. */
export function UpdateDialog() {
  const { t } = useTranslation()
  const { dismiss, install, retry } = useUpdater()
  const open = useUpdateStore((s) => s.dialogOpen)
  const info = useUpdateStore((s) => s.available)
  const phase = useUpdateStore((s) => s.phase)
  const downloaded = useUpdateStore((s) => s.downloaded)
  const total = useUpdateStore((s) => s.total)
  const error = useUpdateStore((s) => s.installError)
  const busy = phase !== "idle"
  const date = formatReleaseDate(info?.date ?? null)
  const percent = total ? Math.min(100, Math.round((downloaded / total) * 100)) : undefined

  return (
    <Dialog open={open && info !== null} onOpenChange={(o) => !o && dismiss()}>
      <DialogContent className="sm:max-w-lg" showCloseButton={false}>
        <DialogHeader>
          <DialogTitle>{t("update.title")}</DialogTitle>
          <DialogDescription>
            {t("update.versions", { current: info?.currentVersion ?? "", next: info?.version ?? "" })}
            {date && ` · ${t("update.date", { date })}`}
          </DialogDescription>
        </DialogHeader>
        {info?.notes && (
          <section aria-label={t("update.notes")} className="flex flex-col gap-1">
            <h3 className="text-xs font-medium text-muted-foreground">{t("update.notes")}</h3>
            <p className="max-h-60 overflow-y-auto rounded-lg border border-border p-3 text-sm whitespace-pre-line break-words">{info.notes}</p>
          </section>
        )}
        {phase === "downloading" && (
          <div className="flex flex-col gap-1.5" role="status">
            <Progress value={percent} aria-label={t("update.downloading")} />
            <p className="text-xs text-muted-foreground">
              {total
                ? t("update.progress", { downloaded: formatBytes(downloaded), total: formatBytes(total) })
                : t("update.progressUnknown", { downloaded: formatBytes(downloaded) })}
            </p>
          </div>
        )}
        {phase === "installing" && (
          <p role="status" className="text-sm">
            {t("update.installing")}
          </p>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {t(updateErrorKey(error))}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={busy} onClick={dismiss}>
            {t("update.later")}
          </Button>
          {error ? (
            <Button onClick={() => void retry()}>{t("update.retry")}</Button>
          ) : (
            <Button disabled={busy} onClick={() => void install()}>
              {t("update.install")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
