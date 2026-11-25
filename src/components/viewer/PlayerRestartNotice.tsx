import { Loader2, RotateCw, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { errorKey } from "@/lib/errors"
import { ExternalOpenButton } from "./ExternalOpenNotice"

interface Props {
  assetId: number
  /** `restarting` = 워커가 죽어 자동 재시작 중, `failed` = 자동 재시작이 모두 실패 */
  status: "restarting" | "failed"
  attempt?: { attempt: number; max: number } | null
  /** 다시 시도 — 실패하면 reject */
  onRetry: () => Promise<void>
}

/**
 * 워커 충돌 안내 — 영상 무대 위에 겹친다 (호출부가 무대의 형제로 두고, 마지막 프레임은 캔버스에 그대로 남는다).
 * 재시작 중: 스피너 + 안내(상태 알림). 재시작 실패: 오류 알림 + [다시 시도] [외부 프로그램으로 열기].
 */
export function PlayerRestartNotice({ assetId, status, attempt, onRetry }: Props) {
  const { t } = useTranslation()
  const [retrying, setRetrying] = useState(false)
  const retry = () => {
    setRetrying(true)
    onRetry()
      .catch((e: unknown) => toast.error(t(errorKey(e))))
      .finally(() => setRetrying(false))
  }
  const box = "absolute inset-x-0 top-1/2 flex -translate-y-1/2 flex-col items-center gap-3 bg-overlay p-4 text-center text-sm text-overlay-foreground"
  if (status === "restarting") {
    const progress = attempt && attempt.max > 1 ? t("viewer.restart.attempt", { attempt: attempt.attempt, max: attempt.max }) : ""
    return (
      <div role="status" aria-live="polite" className={box}>
        <p className="flex items-center gap-2 break-keep">
          <Loader2 aria-hidden="true" className="size-4 animate-spin" />
          {t("viewer.restart.restarting")} {progress}
        </p>
      </div>
    )
  }
  return (
    <div role="alert" className={box}>
      <p className="flex items-center gap-2 break-keep">
        <TriangleAlert aria-hidden="true" className="size-4" />
        {t("viewer.restart.failed")}
      </p>
      <div className="flex flex-wrap items-center justify-center gap-2">
        <Button variant="default" onClick={retry} disabled={retrying} aria-busy={retrying}>
          <RotateCw aria-hidden="true" />
          {t("viewer.restart.retry")}
        </Button>
        <ExternalOpenButton assetId={assetId} />
      </div>
    </div>
  )
}
