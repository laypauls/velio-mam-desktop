import { Download } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useUpdater } from "@/hooks/useUpdater"
import { useUpdateStore } from "@/stores/update"

/** 상단바의 비차단 업데이트 표시 — 새 버전이 있을 때만 보인다. */
export function UpdateIndicator() {
  const { t } = useTranslation()
  const { open } = useUpdater()
  const version = useUpdateStore((s) => s.available?.version)
  if (!version) return null
  const label = t("update.indicator", { version })
  return (
    <Button variant="outline" size="sm" aria-label={label} title={label} onClick={open}>
      <Download aria-hidden="true" />
      <span className="break-keep @max-[486px]:hidden">{t("update.indicatorShort")}</span>
    </Button>
  )
}
