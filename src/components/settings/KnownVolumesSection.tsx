import { HardDrive } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { useForgetVolume, useKnownVolumes, useUnignoreVolume } from "@/hooks/useNewVolumes"
import { errorKey } from "@/lib/errors"
import { formatDate } from "@/lib/format"
import type { KnownVolume } from "@/lib/ipc-contract"

const volumeName = (v: KnownVolume) => v.label ?? v.lastMountPoint ?? v.volumeKey

function ForgetDialog({ volume, onClose }: { volume: KnownVolume | null; onClose: () => void }) {
  const { t } = useTranslation()
  const forget = useForgetVolume()
  const close = () => {
    forget.reset()
    onClose()
  }
  const name = volume ? volumeName(volume) : ""
  return (
    <AlertDialog open={volume !== null} onOpenChange={(o) => !o && !forget.isPending && close()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("volumes.known.forgetTitle", { name })}</AlertDialogTitle>
          <AlertDialogDescription className="break-keep">{t("volumes.known.forgetDescription")}</AlertDialogDescription>
        </AlertDialogHeader>
        {forget.isError && (
          <p role="alert" className="text-xs text-destructive break-keep">
            {t(errorKey(forget.error))}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={forget.isPending}>{t("common.cancel")}</AlertDialogCancel>
          <Button
            variant="destructive"
            aria-busy={forget.isPending}
            disabled={forget.isPending || volume === null}
            onClick={() => volume && forget.mutate(volume.volumeKey, { onSuccess: close })}
          >
            {t("volumes.known.forget")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

function KnownVolumeRow({ volume, onForget }: { volume: KnownVolume; onForget: () => void }) {
  const { t } = useTranslation()
  const unignore = useUnignoreVolume()
  const name = volumeName(volume)
  const ignoredOnly = volume.availability === null
  return (
    <li className="flex items-center gap-2 px-3 py-2">
      <HardDrive aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-sm font-medium break-all">{name}</span>
          {volume.availability !== null && (
            <Badge variant={volume.availability === "online" ? "secondary" : "outline"} className="rounded-sm">
              {t(`volumes.known.state.${volume.availability}`)}
            </Badge>
          )}
          {volume.ignored && (
            <Badge variant="outline" className="rounded-sm">
              {t("volumes.known.state.ignored")}
            </Badge>
          )}
        </div>
        <span className="text-xs text-muted-foreground break-keep">
          {[
            volume.lastMountPoint ?? "-",
            volume.folderCount > 0 ? t("volumes.known.folders", { count: volume.folderCount }) : null,
            volume.availability === "offline" && volume.lastSeenAt !== null
              ? t("volumes.known.lastSeen", { date: formatDate(volume.lastSeenAt) })
              : null,
          ]
            .filter((s) => s !== null)
            .join(" · ")}
        </span>
        {volume.folderCount > 0 && <span className="text-xs text-muted-foreground break-keep">{t("volumes.known.hasFolders")}</span>}
        {unignore.isError && (
          <span role="alert" className="text-xs text-destructive">
            {t(errorKey(unignore.error))}
          </span>
        )}
      </div>
      {volume.folderCount === 0 &&
        (ignoredOnly ? (
          <Button
            size="sm"
            variant="outline"
            aria-label={t("volumes.ignored.askAgainFor", { name })}
            disabled={unignore.isPending}
            onClick={() => unignore.mutate(volume.volumeKey)}
          >
            {t("volumes.ignored.askAgain")}
          </Button>
        ) : (
          <Button size="sm" variant="outline" aria-label={t("volumes.known.forgetFor", { name })} onClick={onForget}>
            {t("volumes.known.forget")}
          </Button>
        ))}
    </li>
  )
}

/** 설정 > 관리 폴더 하단 — 앱이 기억하는 디스크. 관리 폴더가 없는 디스크만 기록을 지울 수 있다(연결 데이터는 건드리지 않는다). 비어 있으면 그리지 않는다. */
export function KnownVolumesSection() {
  const { t } = useTranslation()
  const { data } = useKnownVolumes()
  const [target, setTarget] = useState<KnownVolume | null>(null)
  if (!data || data.length === 0) return null
  return (
    <section aria-labelledby="known-volumes-title" className="flex flex-col gap-2">
      <h4 id="known-volumes-title" className="text-sm font-semibold">
        {t("volumes.known.title")}
      </h4>
      <p className="text-xs text-muted-foreground break-keep">{t("volumes.known.description")}</p>
      <ul aria-labelledby="known-volumes-title" className="flex flex-col divide-y divide-border rounded-lg border border-border">
        {data.map((v) => (
          <KnownVolumeRow key={v.volumeKey} volume={v} onForget={() => setTarget(v)} />
        ))}
      </ul>
      <ForgetDialog volume={target} onClose={() => setTarget(null)} />
    </section>
  )
}
