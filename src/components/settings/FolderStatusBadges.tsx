import { FlaskConical, TriangleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import type { ManagedFolder, ScanStall } from "@/lib/ipc-contract"

/** 원격 접속을 쓰는 감지기 — 연결 상태(● 연결)를 정상일 때도 보여 준다. */
export const REMOTE_DETECTORS: readonly string[] = ["ddp_audit", "fanotify_feed"]

/** 실험 감지기 표시 — 표 한 줄에 맞게 아이콘만 쓰고 접근 가능한 이름을 단다. */
export function ExperimentalMark() {
  const { t } = useTranslation()
  return <FlaskConical role="img" aria-label={t("watch.experimental")} className="size-3.5 shrink-0 text-muted-foreground" />
}

/**
 * 변경 신호원 상태 배지 — 문제가 있으면(`degraded`) 사유를 툴팁·스크린리더 문구로 붙이고,
 * 정상은 원격 접속 감지기에서만 "연결" 로 보여 준다 (실시간·폴링의 정상은 표시할 것이 없다).
 */
export function WatchHealthBadge({ folder }: { folder: ManagedFolder }) {
  const { t } = useTranslation()
  const health = folder.watchHealth
  if (health?.state === "degraded") {
    const reason = t(health.reasonKey, { defaultValue: t("watch.health.unknown") })
    const label = t("watch.health.degradedLabel", { reason })
    return (
      <Badge variant="outline" title={label} className="min-w-0 shrink-0 gap-1 rounded-sm text-destructive">
        <TriangleAlert aria-hidden="true" className="size-3" />
        <span>{t("watch.health.degraded")}</span>
        <span className="sr-only">{reason}</span>
      </Badge>
    )
  }
  if (health?.state === "ok" && folder.watchActive !== null && REMOTE_DETECTORS.includes(folder.watchActive)) {
    return (
      <Badge variant="outline" className="shrink-0 gap-1 rounded-sm">
        <span aria-hidden="true" className="size-1.5 rounded-full bg-primary" />
        {t("watch.health.ok")}
      </Badge>
    )
  }
  return null
}

/** 스캔 멈춤 경과 분 (최소 1) */
export function stallMinutes(stall: ScanStall, now: number): number {
  return Math.max(1, Math.floor((now - stall.since) / 60_000))
}

/**
 * "스캔 멈춤 N분 (붙들린 곳: 경로)" — 폴더 이름 아래 줄에 둔다. 좁은 이름 칸의 배지 안에서는 분·경로가 모두 잘려 보이지 않기 때문이다
 * (멈춤은 드문 예외 상태라 그때만 행이 두 줄이 된다). 경로가 길면 한 줄 안에서 잘리고 전체는 툴팁으로 본다.
 * 응답 없는 공유를 읽는 중일 수 있고 파일 삭제가 아니다.
 */
export function ScanStallLine({ stall }: { stall: ScanStall }) {
  const { t } = useTranslation()
  const minutes = stallMinutes(stall, Date.now())
  const text = t("watch.stall.badge", { minutes })
  const at = t("watch.stall.at", { path: stall.atPath })
  const full = `${text} (${at}) — ${t("watch.stall.hint")}`
  return (
    <p role="status" title={full} aria-label={full} className="flex min-w-0 items-center gap-1 text-xs text-destructive">
      <TriangleAlert aria-hidden="true" className="size-3 shrink-0" />
      <span aria-hidden="true" className="truncate">
        {text} ({at})
      </span>
    </p>
  )
}
