import { ChevronRight } from "lucide-react"
import { useTranslation } from "react-i18next"
import { MediaTypeIcon } from "@/components/gallery/AssetThumb"
import { AvailabilityBadge } from "@/components/gallery/AvailabilityBadge"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { useDisplayName } from "@/hooks/useDisplayName"
import type { AssetDetail, AssetSummary } from "@/lib/ipc-contract"
import { useSelectionStore } from "@/stores"

function LinkList({ title, items }: { title: string; items: AssetSummary[] }) {
  const selectOnly = useSelectionStore((s) => s.selectOnly)
  const { nameOf, tooltipOf } = useDisplayName()
  if (items.length === 0) return null
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
      <ul className="flex flex-col gap-1">
        {items.map((a) => (
          <li key={a.assetId}>
            <button
              type="button"
              onClick={() => selectOnly(a.assetId)}
              className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-sm hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
            >
              <MediaTypeIcon type={a.mediaType} className="size-4 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate" title={tooltipOf(a)}>{nameOf(a)}</span>
              <AvailabilityBadge availability={a.availability} />
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}

/** 중복본·이전 버전 — 접힌 "관련 파일" 한 줄로 모으고, 둘 다 없으면 그리지 않는다. 항목을 누르면 해당 자산으로 이동. */
export function AssetLinks({ detail }: { detail: AssetDetail }) {
  const { t } = useTranslation()
  const dup = detail.duplicates.length
  const prev = detail.previousVersions.length
  if (dup === 0 && prev === 0) return null
  const parts = [t("detail.related"), dup > 0 ? t("detail.relatedDuplicates", { count: dup }) : null, prev > 0 ? t("detail.relatedPrevious", { count: prev }) : null]
  return (
    <Collapsible className="flex flex-col gap-2">
      <CollapsibleTrigger className="group flex items-center gap-1 self-start rounded-sm text-xs text-muted-foreground hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none">
        <ChevronRight aria-hidden="true" className="size-3.5 transition-transform group-data-[state=open]:rotate-90" />
        {parts.filter(Boolean).join(" · ")}
      </CollapsibleTrigger>
      <CollapsibleContent className="flex flex-col gap-3">
        <LinkList title={t("detail.duplicates", { count: dup })} items={detail.duplicates} />
        <LinkList title={t("detail.previous", { count: prev })} items={detail.previousVersions} />
      </CollapsibleContent>
    </Collapsible>
  )
}
