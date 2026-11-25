import { memo } from "react"
import { useTranslation } from "react-i18next"
import { Checkbox } from "@/components/ui/checkbox"
import { Skeleton } from "@/components/ui/skeleton"
import { sectionsInRange } from "@/hooks/galleryWindow/sections"
import { displayName, nameTooltip } from "@/lib/displayName"
import { formatBytes, formatDate, formatDuration } from "@/lib/format"
import { effectiveCapturedAt } from "@/lib/capturedAt"
import type { AssetSummary, CapturedAtFallback } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { modsOf, type CellHandlers } from "./AssetCell"
import { MediaTypeIcon } from "./AssetThumb"
import { AvailabilityBadge } from "./AvailabilityBadge"
import { optionId } from "./optionId"
import type { GalleryGeometry } from "./galleryGeometry"

/** 보이는 영역 위아래로 더 그리는 행 수 */
const OVERSCAN_ROWS = 8
/**
 * 열 템플릿 — 이름 열에 실제 최소 폭(10rem)을 둔다. `minmax(0,1fr)` 이면 좁은 영역에서 이름 열이 0 으로 접혀
 * 머리글이 글자 단위로 줄바꿈되고 유형 아이콘이 옆 열을 덮는다.
 */
export const LIST_GRID_TEMPLATE = "grid-cols-[2rem_minmax(10rem,1fr)_5rem_6.5rem_5rem_6rem_7rem]"
/** 목록 최소 폭 = 고정 열 31.5rem + 이름 최소 10rem + 간격 4.5rem + 좌우 여백 1.5rem. 이보다 좁으면 가로 스크롤한다 */
export const LIST_MIN_W = "min-w-[47.5rem]"
const COLS = `grid ${LIST_GRID_TEMPLATE} ${LIST_MIN_W} items-center gap-3 px-3`

interface Props extends CellHandlers {
  geo: GalleryGeometry
  offset: number
  getItem: (g: number) => AssetSummary | undefined
  selected: ReadonlySet<number>
  activeId: number | null
  titleEditEnabled?: boolean
  capturedAtFallback?: CapturedAtFallback
  /** 컬렉션 순서 정렬 — 이름 앞에 `#n`(전역 위치 + 1) 표시 */
  showOrder?: boolean
  /** 순서 재배치 드래그 가능 */
  draggable?: boolean
}

interface RowProps extends CellHandlers {
  asset: AssetSummary
  selected: boolean
  active: boolean
  titleEditEnabled: boolean
  capturedAtFallback: CapturedAtFallback
  orderNumber?: number
  draggable: boolean
}

const AssetRow = memo(function AssetRow({ asset, selected, active, onSelect, onOpen, titleEditEnabled, capturedAtFallback, orderNumber, draggable }: RowProps) {
  const { t } = useTranslation()
  const name = displayName(asset, titleEditEnabled)
  return (
    <div
      role="option"
      id={optionId(asset.assetId)}
      data-active={active || undefined}
      aria-selected={selected}
      aria-label={name}
      data-asset-id={asset.assetId}
      draggable={draggable}
      onClick={(e) => onSelect(asset.assetId, modsOf(e))}
      onDoubleClick={() => onOpen(asset.assetId)}
      className={cn(COLS, "h-full cursor-pointer border-b border-border text-sm select-none hover:bg-muted/50 aria-selected:bg-accent data-[active]:outline-2 data-[active]:-outline-offset-2 data-[active]:outline-ring")}
    >
      <Checkbox
        checked={selected}
        aria-label={t("gallery.selectItem", { name })}
        onClick={(e) => e.stopPropagation()}
        onCheckedChange={() => onSelect(asset.assetId, { ctrl: true, shift: false })}
      />
      <span className="flex min-w-0 items-center gap-2">
        {orderNumber !== undefined && (
          <span title={t("order.number", { n: orderNumber })} className="shrink-0 text-xs text-muted-foreground tabular-nums">
            #{orderNumber}
          </span>
        )}
        <MediaTypeIcon type={asset.mediaType} className="size-4 shrink-0 text-muted-foreground" />
        <span className="truncate" title={nameTooltip(asset, titleEditEnabled)}>{name}</span>
      </span>
      <span className="text-muted-foreground">{t(`asset.type.${asset.mediaType}`)}</span>
      <span className="text-muted-foreground">{formatDate(effectiveCapturedAt(asset, capturedAtFallback).at)}</span>
      <span className="text-right text-muted-foreground tabular-nums">{formatDuration(asset.durationMs)}</span>
      <span className="text-right text-muted-foreground tabular-nums">{formatBytes(asset.fileSize)}</span>
      <span>
        {asset.availability === "online" ? (
          <span className="text-muted-foreground">{t("asset.availability.online")}</span>
        ) : (
          <AvailabilityBadge availability={asset.availability} />
        )}
      </span>
    </div>
  )
})

/** 목록 보기 (D1) — 격자와 같은 오프셋 모델(고정 행 높이). 열 머리글은 스크롤하지 않는 고정 영역이다. */
export function VirtualList({ geo, offset, getItem, selected, activeId, onSelect, onOpen, titleEditEnabled = false, capturedAtFallback = "modified", showOrder = false, draggable = false }: Props) {
  const { t } = useTranslation()
  const { model, padTop, rowH } = geo
  const y0 = offset - padTop - OVERSCAN_ROWS * rowH
  const y1 = offset - padTop + geo.viewportH + OVERSCAN_ROWS * rowH
  const section = sectionsInRange(model, y0, y1)[0]
  const first = section ? Math.max(0, Math.floor((y0 - section.startY) / rowH)) : 0
  const last = section ? Math.min(section.rows - 1, Math.floor((y1 - section.startY) / rowH)) : -1

  return (
    <div role="presentation" className={cn("relative h-full w-full overflow-hidden", LIST_MIN_W)}>
      <div
        aria-hidden="true"
        className={cn(COLS, "absolute inset-x-0 top-0 z-10 border-b border-border bg-background text-xs whitespace-nowrap font-medium text-muted-foreground")}
        style={{ height: padTop }}
      >
        <span />
        <span>{t("list.col.name")}</span>
        <span>{t("list.col.type")}</span>
        <span>{t("list.col.captured")}</span>
        <span className="text-right">{t("list.col.duration")}</span>
        <span className="text-right">{t("list.col.size")}</span>
        <span>{t("list.col.status")}</span>
      </div>
      {Array.from({ length: Math.max(0, last - first + 1) }, (_, k) => {
        const g = (section?.start ?? 0) + first + k
        const asset = getItem(g)
        return (
          <div
            key={asset ? asset.assetId : `sk${g}`}
            role="presentation"
            className="absolute top-0 left-0 w-full"
            style={{ height: rowH, transform: `translateY(${padTop - offset + (section?.startY ?? 0) + (first + k) * rowH}px)` }}
          >
            {asset ? (
              <AssetRow asset={asset} selected={selected.has(asset.assetId)} active={asset.assetId === activeId} onSelect={onSelect} onOpen={onOpen} titleEditEnabled={titleEditEnabled} capturedAtFallback={capturedAtFallback} orderNumber={showOrder ? g + 1 : undefined} draggable={draggable} />
            ) : (
              <div data-placeholder="true" aria-hidden="true" className={cn(COLS, "h-full border-b border-border")}>
                <span />
                <Skeleton className="h-4 w-2/3" />
              </div>
            )}
          </div>
        )
      })}
    </div>
  )
}
