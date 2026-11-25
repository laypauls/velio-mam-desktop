import { CircleCheck, TriangleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"
import { REMOTE_DETECTORS } from "@/components/settings/FolderStatusBadges"
import { formatDateTime } from "@/lib/format"
import type { ManagedFolder } from "@/lib/ipc-contract"

/**
 * 변경 신호원 상태 한 줄 — 문제가 있으면 사유와 시작 시각, 원격 접속 감지기의 정상은 마지막 신호 시각.
 * 그 밖의 정상(실시간·폴링·재스캔만)은 보여 줄 것이 없어 그리지 않는다. 저장된 폴더 상태 기준이다.
 */
export function WatchHealthLine({ folder }: { folder: ManagedFolder }) {
  const { t } = useTranslation()
  const health = folder.watchHealth
  if (health === undefined || folder.watchActive === null) return null
  if (health.state === "degraded") {
    const reason = t(health.reasonKey, { defaultValue: t("watch.health.unknown") })
    return (
      <p role="status" className="flex items-start gap-1.5 text-xs break-keep text-destructive">
        <TriangleAlert aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
        <span>
          {t("watch.health.lineDegraded", { reason, at: formatDateTime(health.since) })} {t("watch.health.rescanCovers")}
        </span>
      </p>
    )
  }
  if (!REMOTE_DETECTORS.includes(folder.watchActive)) return null
  return (
    <p role="status" className="flex items-start gap-1.5 text-xs break-keep text-muted-foreground">
      <CircleCheck aria-hidden="true" className="mt-0.5 size-3.5 shrink-0 text-primary" />
      <span>
        {health.lastSignalAt === null
          ? t("watch.health.lineOkNoSignal")
          : t("watch.health.lineOk", { at: formatDateTime(health.lastSignalAt) })}
      </span>
    </p>
  )
}
