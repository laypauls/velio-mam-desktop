import { PanelRightClose, PanelRightOpen } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useSelectionStore, useViewStore } from "@/stores"
import { DetailBody } from "./DetailBody"
import { MultiSelectDetail } from "./MultiSelectDetail"

/** D8: 우측 고정 상세 패널 (접기 가능). 활성 자산(마지막 클릭)을 보여준다. */
export function DetailPanel() {
  const { t } = useTranslation()
  const open = useViewStore((s) => s.detailOpen)
  const setOpen = useViewStore((s) => s.setDetailOpen)
  const activeId = useSelectionStore((s) => s.activeId)
  const selectedIds = useSelectionStore((s) => s.ids)

  if (!open) {
    return (
      <div className="flex shrink-0 border-l border-border p-1">
        <Button size="icon-sm" variant="ghost" aria-label={t("detail.openPanel")} onClick={() => setOpen(true)}>
          <PanelRightOpen aria-hidden="true" />
        </Button>
      </div>
    )
  }
  return (
    <aside aria-label={t("detail.panel")} className="flex w-80 shrink-0 flex-col border-l border-border">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <h2 className="text-sm font-medium">{t("detail.title")}</h2>
        <Button size="icon-sm" variant="ghost" aria-label={t("detail.closePanel")} onClick={() => setOpen(false)}>
          <PanelRightClose aria-hidden="true" />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto p-3">
        {selectedIds.size > 1 ? (
          <MultiSelectDetail ids={selectedIds} activeId={activeId} />
        ) : activeId === null ? (
          <p className="text-sm break-keep text-muted-foreground">{t("detail.none")}</p>
        ) : (
          <DetailBody assetId={activeId} />
        )}
      </div>
    </aside>
  )
}
