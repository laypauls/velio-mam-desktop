import { CircleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { errorKey } from "@/lib/errors"

/** 인라인 오류 + 다시 시도 (toast 보다 인라인 우선). */
export function InlineError({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  const { t } = useTranslation()
  return (
    <div role="alert" className="flex items-center gap-3 border-b border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      <CircleAlert aria-hidden="true" className="size-4 shrink-0" />
      <span className="flex-1 break-keep">{t("gallery.error", { reason: t(errorKey(error)) })}</span>
      <Button size="sm" variant="outline" onClick={onRetry}>
        {t("common.retry")}
      </Button>
    </div>
  )
}
