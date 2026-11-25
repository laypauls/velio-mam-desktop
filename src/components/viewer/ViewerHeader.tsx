import { X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { MediaTypeIcon } from "@/components/gallery/AssetThumb"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useDisplayName } from "@/hooks/useDisplayName"
import { formatDuration } from "@/lib/format"
import type { AssetDetail, MediaType } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"

export const TYPE_COLOR: Record<MediaType, string> = {
  image: "text-media-image",
  video: "text-media-video",
  audio: "text-media-audio",
}

/** 해상도·fps·길이 — 값이 있는 것만 `·` 로 잇는다. */
export function viewerMeta(a: AssetDetail): string {
  const parts: string[] = []
  if (a.mediaType !== "audio" && a.width !== null && a.height !== null) parts.push(`${a.width}×${a.height}`)
  if (a.mediaType === "video" && a.fps !== null && a.fps > 0) parts.push(`${Number(a.fps.toFixed(2))}fps`)
  if (a.mediaType !== "image" && a.durationMs !== null) parts.push(formatDuration(a.durationMs))
  return parts.join(" · ")
}

/** 보기 영역 머리글 — 유형 배지, 표시 이름, 해상도/fps/길이, 닫기. 자산 상세가 아직 없으면 자리표시. */
export function ViewerHeader({ asset, onClose }: { asset: AssetDetail | undefined; onClose: () => void }) {
  const { t } = useTranslation()
  const { nameOf, tooltipOf } = useDisplayName()
  const meta = asset ? viewerMeta(asset) : ""
  return (
    <header className="flex flex-col gap-0.5 border-b border-border px-3 py-2">
      <div className="flex items-center gap-2">
        {asset ? (
          <Badge variant="outline" className={cn("rounded-sm", TYPE_COLOR[asset.mediaType])}>
            <MediaTypeIcon type={asset.mediaType} />
            {t(`asset.type.${asset.mediaType}`)}
          </Badge>
        ) : (
          <Skeleton className="h-5 w-14" />
        )}
        <h2 className="min-w-0 flex-1 truncate text-sm font-medium" title={asset ? tooltipOf(asset) : undefined}>
          {asset ? nameOf(asset) : ""}
        </h2>
        <Button size="icon-sm" variant="ghost" aria-label={t("viewer.close")} onClick={onClose}>
          <X aria-hidden="true" />
        </Button>
      </div>
      {meta !== "" && <p className="text-xs text-muted-foreground tabular-nums">{meta}</p>}
    </header>
  )
}
