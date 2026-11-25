import { Check, CircleAlert, Heart, Maximize2, Play, WifiOff } from "lucide-react"
import type { MouseEvent, ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { RatingInput } from "@/components/rating/RatingInput"
import { formatDuration } from "@/lib/format"
import type { AssetSummary, GalleryCardDisplay } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { MediaTypeIcon } from "./AssetThumb"
import { COMPACT_CHIP_CELL } from "./cardDisplay"

interface Props {
  asset: AssetSummary
  display: GalleryCardDisplay
  cellWidth: number
  selected: boolean
  /** 하나 이상 선택돼 있다 — 체크 "호버 시" 모드에서 모든 체크를 보인다 */
  selecting: boolean
  onToggleCheck: (id: number) => void
  onToggleFavorite: (id: number, favorite: boolean) => void
  onSetRating?: (id: number, rating: number) => void
  onOpen: (id: number) => void
}

/**
 * 오버레이 버튼 공통 — 클릭·더블클릭이 셀의 선택/열기로 번지지 않게 막고, 마우스로 눌러도 격자(listbox)가
 * 포커스를 유지해 키보드 탐색이 이어진다. `tabIndex=-1` 이라 roving 탐색에 끼지 않는다.
 */
function OverlayButton({ onPress, className, children, ...aria }: { onPress: () => void; className: string; children: ReactNode; "aria-label": string; "aria-pressed"?: boolean }) {
  const press = (e: MouseEvent<HTMLButtonElement>) => {
    e.stopPropagation()
    e.currentTarget.closest<HTMLElement>("[role='listbox']")?.focus({ preventScroll: true })
    onPress()
  }
  return (
    <button
      type="button"
      tabIndex={-1}
      onClick={press}
      onDoubleClick={(e) => e.stopPropagation()}
      onMouseDown={(e) => e.preventDefault()}
      className={className}
      {...aria}
    >
      {children}
    </button>
  )
}

/** 호버 별점 입력을 그리는 최소 카드 폭 — 더 좁으면 체크·하트와 겹친다 */
const RATING_OVERLAY_CELL = 170

const CHIP = "inline-flex h-5 items-center gap-1 rounded-sm bg-overlay px-1.5 text-[11px] font-semibold tabular-nums text-overlay-foreground"
const ROUND_BTN = "grid place-items-center rounded-full bg-overlay text-overlay-foreground transition-opacity"

/** 썸네일 위 네 귀퉁이 — 왼쪽 위 선택 체크 / 오른쪽 위 즐겨찾기 / 왼쪽 아래 상태 / 오른쪽 아래 길이(호버: 큰 보기). */
export function AssetCardOverlay({ asset, display, cellWidth, selected, selecting, onToggleCheck, onToggleFavorite, onSetRating, onOpen }: Props) {
  const { t } = useTranslation()
  const { assetId, availability, mediaType, durationMs } = asset
  const showDuration = display.duration && mediaType !== "image" && durationMs !== null
  const StatusIcon = availability === "offline" ? WifiOff : CircleAlert
  const checkVisible = selected || display.check === "always" || (display.check === "hover" && selecting)

  return (
    <>
      {display.check !== "hidden" && (
        <div className="absolute top-1.5 left-1.5">
          <OverlayButton
            aria-label={selected ? t("card.deselect") : t("card.select")}
            aria-pressed={selected}
            onPress={() => onToggleCheck(assetId)}
            className={cn(
              "grid size-5.5 place-items-center rounded-full border-2 transition-opacity",
              selected ? "border-primary bg-primary text-primary-foreground" : "border-overlay-foreground bg-overlay/40 text-transparent",
              checkVisible ? "opacity-100" : "opacity-0 group-hover:opacity-100"
            )}
          >
            <Check aria-hidden="true" className="size-3.5" strokeWidth={3} />
          </OverlayButton>
        </div>
      )}
      {display.rating && onSetRating && cellWidth >= RATING_OVERLAY_CELL && (
        <div className={cn("absolute top-1.5 flex h-6.5 items-center rounded-full bg-overlay px-1.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100", display.favorite ? "right-9" : "right-1.5")}>
          <RatingInput value={asset.rating} onChange={(v) => onSetRating(assetId, v)} tabbable={false} keepGridFocus onOverlay starClassName="size-3.5" />
        </div>
      )}
      {display.favorite && (
        <div className="absolute top-1.5 right-1.5">
          <OverlayButton
            aria-label={asset.favorite ? t("favorite.off") : t("favorite.on")}
            onPress={() => onToggleFavorite(assetId, !asset.favorite)}
            className={cn(ROUND_BTN, "size-6.5", asset.favorite ? "text-favorite opacity-100" : "opacity-0 group-hover:opacity-100")}
          >
            <Heart aria-hidden="true" className={cn("size-3.5", asset.favorite && "fill-current")} />
          </OverlayButton>
        </div>
      )}
      {availability !== "online" && (
        <div className="absolute bottom-1.5 left-1.5">
          <span className={cn(CHIP, availability === "missing" && "bg-destructive")} title={t(`asset.availability.${availability}`)}>
            <StatusIcon aria-hidden="true" className="size-3" />
            {cellWidth >= COMPACT_CHIP_CELL && <span className="break-keep">{t(`asset.availability.${availability}`)}</span>}
          </span>
        </div>
      )}
      {showDuration && (
        <div className={cn("absolute right-1.5 bottom-1.5", display.quickActions && "transition-opacity group-hover:opacity-0")}>
          <span className={CHIP}>
            {mediaType === "video" ? <Play aria-hidden="true" className="size-3 fill-current" /> : <MediaTypeIcon type="audio" className="size-3" />}
            {formatDuration(durationMs)}
          </span>
        </div>
      )}
      {display.quickActions && (
        <div className="absolute right-1.5 bottom-1.5">
          <OverlayButton
            aria-label={t("card.preview")}
            onPress={() => onOpen(assetId)}
            className={cn(ROUND_BTN, "size-6.5 rounded-md opacity-0 group-hover:opacity-100")}
          >
            <Maximize2 aria-hidden="true" className="size-3.5" />
          </OverlayButton>
        </div>
      )}
    </>
  )
}
