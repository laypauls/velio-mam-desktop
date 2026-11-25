import { FileAudio, FileImage, FileVideo, ImageOff, Loader } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useApi } from "@/lib/api"
import type { AssetSummary, MediaType, ThumbSize } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"

const TYPE_ICON = { image: FileImage, video: FileVideo, audio: FileAudio } as const

export function MediaTypeIcon({ type, className }: { type: MediaType; className?: string }) {
  const Icon = TYPE_ICON[type]
  return <Icon className={className} aria-hidden="true" />
}

interface Props {
  asset: Pick<AssetSummary, "assetId" | "mediaType" | "thumbState">
  size: ThumbSize
  className?: string
}

/** D5: 썸네일이 done 이면 지연 로딩 img, 아니면 유형 아이콘 자리표시. 음성은 항상 아이콘. */
export function AssetThumb({ asset, size, className }: Props) {
  const api = useApi()
  const { t } = useTranslation()
  const [failedId, setFailedId] = useState<number | null>(null)
  const showImg = asset.thumbState === "done" && asset.mediaType !== "audio" && failedId !== asset.assetId

  return (
    <div className={cn("flex size-full items-center justify-center bg-muted text-muted-foreground", className)}>
      {showImg ? (
        <img
          src={api.thumbUrl(asset.assetId, size)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          className="size-full object-cover"
          onError={() => setFailedId(asset.assetId)}
        />
      ) : asset.thumbState === "pending" ? (
        <Loader className="size-6" role="img" aria-label={t("asset.thumb.pending")} />
      ) : asset.thumbState === "failed" ? (
        <ImageOff className="size-6" role="img" aria-label={t("asset.thumb.failed")} />
      ) : (
        <MediaTypeIcon type={asset.mediaType} className="size-8" />
      )}
    </div>
  )
}
