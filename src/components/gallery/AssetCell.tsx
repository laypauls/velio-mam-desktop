import { memo, type MouseEvent } from "react"
import { useTranslation } from "react-i18next"
import type { ClickMods } from "@/stores/selection"
import { displayName } from "@/lib/displayName"
import { formatDuration } from "@/lib/format"
import type { AssetSummary, CapturedAtFallback, GalleryCardDisplay } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { AssetCardInfo } from "./AssetCardInfo"
import { AssetCardOverlay } from "./AssetCardOverlay"
import { AssetThumb } from "./AssetThumb"
import { cardInfoLines } from "./cardDisplay"
import { optionId } from "./optionId"

export interface CellHandlers {
  onSelect: (id: number, mods: ClickMods) => void
  onOpen: (id: number) => void
}

interface Props extends CellHandlers {
  asset: AssetSummary
  selected: boolean
  /** 키보드 포커스(cursor) 항목 */
  active?: boolean
  display: GalleryCardDisplay
  /** 카드 폭(px) — 140/160 미만 축약 규칙에 쓴다 */
  cellWidth: number
  /** 하나 이상 선택돼 있다 */
  selecting: boolean
  onToggleFavorite: (id: number, favorite: boolean) => void
  /** 호버 오버레이의 별점 입력 — 생략하면 입력을 그리지 않는다 */
  onSetRating?: (id: number, rating: number) => void
  /** 카드 빠른 동작(크게 보기)은 더블클릭 설정과 무관하게 항상 큰 보기 — 생략하면 `onOpen` */
  onPreview?: (id: number) => void
  titleEditEnabled?: boolean
  capturedAtFallback?: CapturedAtFallback
  /** 컬렉션 안 순번(`#n`) — 컬렉션 순서 정렬일 때만 넘긴다 */
  orderNumber?: number
  /** 순서 재배치 드래그가 가능한 상태 */
  draggable?: boolean
}

const CHECK_CLICK: ClickMods = { ctrl: true, shift: false }

/** 스크린리더용 이름 — 파일명, 유형, 길이, 접근 상태 */
function useCardLabel(asset: AssetSummary, titleEditEnabled: boolean): string {
  const { t } = useTranslation()
  const parts = [displayName(asset, titleEditEnabled), t(`asset.type.${asset.mediaType}`)]
  if (asset.mediaType !== "image" && asset.durationMs !== null) parts.push(formatDuration(asset.durationMs))
  if (asset.availability !== "online") parts.push(t(`asset.availability.${asset.availability}`))
  return parts.join(", ")
}

/** 갤러리 한 칸 — 정사각 썸네일(귀퉁이 오버레이) + 아래 정보 영역. 높이는 격자가 `cell + infoHeight` 로 준다. */
export const AssetCell = memo(function AssetCell({
  asset,
  selected,
  active = false,
  display,
  cellWidth,
  selecting,
  onSelect,
  onOpen,
  onToggleFavorite,
  onSetRating,
  onPreview,
  titleEditEnabled = false,
  capturedAtFallback = "modified",
  orderNumber,
  draggable = false,
}: Props) {
  const { t } = useTranslation()
  const label = useCardLabel(asset, titleEditEnabled)
  const hasInfo = cardInfoLines(display, cellWidth) > 0
  const unavailable = asset.availability !== "online"
  return (
    <div
      role="option"
      id={optionId(asset.assetId)}
      data-active={active || undefined}
      aria-selected={selected}
      aria-label={label}
      data-asset-id={asset.assetId}
      draggable={draggable}
      onClick={(e) => onSelect(asset.assetId, modsOf(e))}
      onDoubleClick={() => onOpen(asset.assetId)}
      className={cn(
        "group relative flex size-full cursor-pointer flex-col overflow-hidden rounded-lg border border-border bg-card select-none",
        "aria-selected:border-primary aria-selected:bg-primary/10 aria-selected:ring-2 aria-selected:ring-primary",
        "data-[active]:outline-2 data-[active]:outline-offset-2 data-[active]:outline-ring"
      )}
    >
      <div className={cn("relative aspect-square w-full shrink-0 overflow-hidden", hasInfo && "border-b border-border")}>
        <AssetThumb asset={asset} size={320} className={cn(unavailable && "opacity-50 grayscale")} />
        <AssetCardOverlay
          asset={asset}
          display={display}
          cellWidth={cellWidth}
          selected={selected}
          selecting={selecting}
          onToggleCheck={(id) => onSelect(id, CHECK_CLICK)}
          onToggleFavorite={onToggleFavorite}
          onSetRating={onSetRating}
          onOpen={onPreview ?? onOpen}
        />
        {orderNumber !== undefined && (
          <span
            title={t("order.number", { n: orderNumber })}
            className="absolute top-1.5 left-9 inline-flex h-5 items-center rounded-sm bg-overlay px-1.5 text-[11px] font-semibold tabular-nums text-overlay-foreground"
          >
            #{orderNumber}
          </span>
        )}
      </div>
      {hasInfo && <AssetCardInfo asset={asset} display={display} cellWidth={cellWidth} titleEditEnabled={titleEditEnabled} capturedAtFallback={capturedAtFallback} />}
    </div>
  )
})

export function modsOf(e: MouseEvent): ClickMods {
  return { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey }
}
