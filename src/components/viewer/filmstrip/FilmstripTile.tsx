import { CircleAlert, Heart, Star, WifiOff } from "lucide-react"
import { memo, type MouseEvent } from "react"
import { useTranslation } from "react-i18next"
import { AssetThumb } from "@/components/gallery/AssetThumb"
import { displayName } from "@/lib/displayName"
import { formatDuration } from "@/lib/format"
import type { AssetSummary } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"

const CHIP = "inline-flex h-4 shrink-0 whitespace-nowrap items-center gap-0.5 rounded-sm bg-overlay px-1 text-[10px] font-semibold tabular-nums text-overlay-foreground"

export const tileId = (assetId: number) => `filmstrip-opt-${assetId}`

interface Props {
  asset: AssetSummary
  /** 전역 0-기준 위치와 전체 수 — 스크린리더가 가상화된 목록에서 위치를 안다 */
  pos: number
  total: number
  width: number
  height: number
  /** 지금 보기가 열어 둔 항목 */
  current: boolean
  /** Ctrl/Shift 선택에 든 항목 */
  selected: boolean
  titleEditEnabled: boolean
  onClick: (e: MouseEvent<HTMLDivElement>, id: number) => void
}

/** 스트립 타일 — 썸네일 + 즐겨찾기·별점(왼쪽 위) + 길이(오른쪽 아래) + 오프라인 빗금. 현재 항목은 강한 링과 `aria-selected`. */
export const FilmstripTile = memo(function FilmstripTile({ asset, pos, total, width, height, current, selected, titleEditEnabled, onClick }: Props) {
  const { t } = useTranslation()
  const name = displayName(asset, titleEditEnabled)
  const unavailable = asset.availability !== "online"
  const showDuration = asset.mediaType !== "image" && asset.durationMs !== null
  const label = [name, t(`asset.type.${asset.mediaType}`), showDuration ? formatDuration(asset.durationMs ?? 0) : null, unavailable ? t(`asset.availability.${asset.availability}`) : null]
    .filter(Boolean)
    .join(", ")
  const StatusIcon = asset.availability === "offline" ? WifiOff : CircleAlert
  return (
    <div
      role="option"
      id={tileId(asset.assetId)}
      aria-selected={current}
      aria-label={label}
      aria-setsize={total}
      aria-posinset={pos + 1}
      data-current={current || undefined}
      data-multi-selected={(selected && !current) || undefined}
      data-unavailable={unavailable || undefined}
      title={name}
      onClick={(e) => onClick(e, asset.assetId)}
      // 크기는 스트립 높이·가로세로비로 계산되는 동적 값
      style={{ width, height }}
      className={cn(
        "relative shrink-0 cursor-pointer overflow-hidden rounded-md border-2 border-transparent select-none",
        selected && !current && "border-primary/50",
        current && "border-primary ring-2 ring-primary ring-offset-1 ring-offset-background"
      )}
    >
      <AssetThumb asset={asset} size={320} className="rounded-sm" />
      {unavailable && (
        <div
          data-testid="filmstrip-offline"
          className="absolute inset-0 grid place-items-center bg-[repeating-linear-gradient(45deg,transparent_0_6px,var(--color-background)_6px_12px)] text-muted-foreground"
        >
          <StatusIcon aria-hidden="true" className="size-4" />
        </div>
      )}
      <div className="absolute top-1 left-1 z-10 flex gap-0.5">
        {asset.favorite && (
          <span className={CHIP}>
            <Heart aria-hidden="true" className="size-2.5 fill-current" />
          </span>
        )}
        {asset.rating > 0 && (
          <span className={CHIP}>
            <Star aria-hidden="true" className="size-2.5 fill-current" />
            {asset.rating}
          </span>
        )}
      </div>
      {showDuration && <span className={cn(CHIP, "absolute right-0.5 bottom-0.5 z-10", width < 56 && "px-0.5 text-[9px]")}>{formatDuration(asset.durationMs ?? 0)}</span>}
    </div>
  )
})
