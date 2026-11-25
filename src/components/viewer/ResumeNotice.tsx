import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { formatDuration } from "@/lib/format"

/** 이어 재생 알림이 떠 있는 시간 (D10) */
export const RESUME_NOTICE_MS = 3000

/**
 * 이어서 재생했을 때 영상 위에 3초간 뜨는 "{시각} 부터 이어서 재생 · [처음부터]" 알림 (부모는 `relative`).
 * `restoredMs` 가 null 이면 아무것도 그리지 않는다. 눌러서 처음으로 가면 바로 사라진다.
 * 부모의 클릭(재생/정지 토글)으로 번지지 않게 클릭을 여기서 막는다.
 */
export function ResumeNotice({ restoredMs, onRestart }: { restoredMs: number | null; onRestart: () => void }) {
  const { t } = useTranslation()
  const [visible, setVisible] = useState(false)
  useEffect(() => {
    if (restoredMs === null) {
      setVisible(false)
      return
    }
    setVisible(true)
    const id = setTimeout(() => setVisible(false), RESUME_NOTICE_MS)
    return () => clearTimeout(id)
  }, [restoredMs])
  if (!visible || restoredMs === null) return null
  return (
    <div
      role="status"
      onClick={(e) => e.stopPropagation()}
      className="absolute bottom-4 left-1/2 z-10 flex max-w-[90%] -translate-x-1/2 items-center gap-2 rounded-xl border border-border bg-popover/90 px-3 py-1.5 text-sm text-popover-foreground shadow-md backdrop-blur-sm"
    >
      <span className="break-keep tabular-nums">{t("viewer.resume.notice", { time: formatDuration(restoredMs) })}</span>
      <span aria-hidden="true" className="text-muted-foreground">
        ·
      </span>
      <Button
        size="sm"
        variant="link"
        className="h-auto p-0"
        onClick={() => {
          setVisible(false)
          onRestart()
        }}
      >
        {t("viewer.resume.restart")}
      </Button>
    </div>
  )
}
