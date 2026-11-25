import { MoreHorizontal } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip"
import { useFitCount } from "@/hooks/useFitCount"
import type { Availability } from "@/lib/ipc-contract"
import { ReasonTooltip } from "./ReasonTooltip"
import { useAssetActionItems } from "./useAssetActionItems"

export { unavailableReason } from "./useAssetActionItems"

/** icon-sm(size-8) 폭과 행의 gap-2 — 인라인에 들어갈 개수 계산용 */
const ITEM_PX = 32
const GAP_PX = 8

interface Props {
  assetId: number
  availability: Availability
}

/** 상세 패널 작업 줄: 아이콘 전용 버튼을 폭이 되는 만큼 인라인으로 두고, 넘치는 항목만 ⋯ 메뉴에 담는다. */
export function AssetActions({ assetId, availability }: Props) {
  const { t } = useTranslation()
  const items = useAssetActionItems(assetId, availability)
  const [rowRef, visible] = useFitCount(items.length, ITEM_PX, GAP_PX)
  const inline = items.slice(0, visible)
  const overflow = items.slice(visible)
  const firstOriginal = overflow.findIndex((i) => i.group === "original")

  return (
    <div ref={rowRef} className="flex items-center gap-2">
      {inline.map((item) => (
        <ReasonTooltip key={item.id} reason={item.reason} label={item.label}>
          <Button
            size="icon-sm"
            variant="outline"
            className={item.destructive ? "text-destructive" : undefined}
            aria-label={item.label}
            disabled={item.disabled}
            onClick={item.run}
          >
            <item.icon aria-hidden="true" />
          </Button>
        </ReasonTooltip>
      ))}
      {overflow.length > 0 && (
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger asChild>
              <DropdownMenuTrigger asChild>
                <Button size="icon-sm" variant="outline" className="ml-auto" aria-label={t("detail.moreActions")}>
                  <MoreHorizontal aria-hidden="true" />
                </Button>
              </DropdownMenuTrigger>
            </TooltipTrigger>
            <TooltipContent className="break-keep">{t("detail.moreActions")}</TooltipContent>
          </Tooltip>
          <DropdownMenuContent align="end" className="min-w-56">
            {overflow.map((item, idx) => (
              <div key={item.id} className="contents">
                {idx === firstOriginal && idx > 0 && <DropdownMenuSeparator />}
                <DropdownMenuItem variant={item.destructive ? "destructive" : "default"} disabled={item.disabled} onSelect={item.run}>
                  <item.icon aria-hidden="true" />
                  {item.label}
                </DropdownMenuItem>
              </div>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
      )}
    </div>
  )
}
