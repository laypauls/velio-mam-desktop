import { Info } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useProtectStore } from "@/stores"

/** D3: 의미 검색을 키워드로 처리했을 때(모델 미준비 등) 안내. */
export function DegradedNotice() {
  const { t } = useTranslation()
  const openSettings = useProtectStore((s) => s.openSettings)
  return (
    <div role="status" className="flex items-center gap-2 border-b border-border bg-muted/40 px-4 py-2 text-sm">
      <Info aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
      <span className="break-keep">{t("search.degraded")}</span>
      <Button size="xs" variant="outline" className="ml-auto" onClick={() => openSettings("ai-local")}>
        {t("search.mode.openSettings")}
      </Button>
    </div>
  )
}
