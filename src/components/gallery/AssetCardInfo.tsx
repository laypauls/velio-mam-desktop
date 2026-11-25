import { useTranslation } from "react-i18next"
import { displayName, nameTooltip, splitFileName } from "@/lib/displayName"
import { formatBytes, formatDateCompact } from "@/lib/format"
import { effectiveCapturedAt } from "@/lib/capturedAt"
import type { AssetSummary, CapturedAtFallback, GalleryCardDisplay, MediaType } from "@/lib/ipc-contract"
import { RatingStars } from "@/components/rating/RatingStars"
import { cn } from "@/lib/utils"
import { AiStateBadge } from "./AiStateBadge"
import { MediaTypeIcon } from "./AssetThumb"
import { NARROW_CELL, hasMetaLine, hasNameLine } from "./cardDisplay"

const TYPE_COLOR: Record<MediaType, string> = {
  image: "text-media-image",
  video: "text-media-video",
  audio: "text-media-audio",
}

interface Props {
  asset: AssetSummary
  display: GalleryCardDisplay
  cellWidth: number
  titleEditEnabled?: boolean
  /** 촬영일이 없을 때 대체(설정) — 기본 수정일 */
  capturedAtFallback?: CapturedAtFallback
}

/** 썸네일 아래 정보 영역 — 1줄: 유형 아이콘·파일명·확장자 칩, 2줄: 상태·해상도·크기·날짜. */
export function AssetCardInfo({ asset, display, cellWidth, titleEditEnabled = false, capturedAtFallback = "modified" }: Props) {
  const { t } = useTranslation()
  const { info } = display
  const { mediaType, availability } = asset
  // 제목이 표시되면 제목에는 확장자가 없으므로 분리하지 않고, 확장자 칩은 실제 파일명에서 가져온다
  const shown = displayName(asset, titleEditEnabled)
  const [fileBase, ext] = splitFileName(asset.fileName)
  const base = shown === asset.fileName ? fileBase : shown
  const showExt = info.ext && cellWidth >= NARROW_CELL && ext !== ""

  const meta: string[] = []
  if (hasMetaLine(display, cellWidth, mediaType)) {
    if (info.resolution && mediaType !== "audio" && asset.width !== null && asset.height !== null) meta.push(`${asset.width}×${asset.height}`)
    if (info.size) meta.push(formatBytes(asset.fileSize))
    if (info.date) {
      const at = display.dateBy === "captured" ? effectiveCapturedAt(asset, capturedAtFallback).at : asset.modifiedAt
      if (at !== null) meta.push(formatDateCompact(at))
    }
  }
  // 메타 줄 유무는 유형과 무관하게 정한다(격자 행 높이와 같은 규칙) — 음성에서 해상도만 켠 경우처럼 값이 비어도 줄은 유지하고,
  // 상태(끌 수 없음)는 그 줄 맨 앞에 붙인다
  const showMeta = hasMetaLine(display, cellWidth)
  // 별점은 meta 줄 앞에 붙인다(줄 수·행 높이는 그대로). 미평가는 숨긴다
  const showStars = display.rating && asset.rating > 0
  const status = availability === "online" ? null : t(`asset.availability.${availability}`)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-hidden px-2 py-[7px]">
      {hasNameLine(display, cellWidth) && (
        <div className="flex h-[18px] min-w-0 items-center gap-1.5">
          {info.typeIcon && (
            <span className="relative flex shrink-0">
              <MediaTypeIcon type={mediaType} className={cn("size-4", TYPE_COLOR[mediaType])} />
              <AiStateBadge state={asset.aiState} />
            </span>
          )}
          {info.name && (
            <span className="min-w-0 flex-1 truncate text-xs font-semibold" title={nameTooltip(asset, titleEditEnabled)}>
              {showExt ? base : shown}
            </span>
          )}
          {showExt && (
            <span className="shrink-0 rounded-sm border border-border px-1 text-[10px] leading-[15px] font-semibold text-muted-foreground">{ext}</span>
          )}
        </div>
      )}
      {showMeta && (
        <div className="flex h-[18px] items-center gap-1.5 text-[11.5px] leading-[18px] text-muted-foreground tabular-nums">
          {showStars && <RatingStars value={asset.rating} className="shrink-0" />}
          <span className="min-w-0 truncate">
            {status !== null && <span className="font-semibold text-destructive">{status}</span>}
            {status !== null && meta.length > 0 && " · "}
            {meta.join(" · ")}
          </span>
        </div>
      )}
    </div>
  )
}
