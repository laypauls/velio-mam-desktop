import { useQuery } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"
import { Skeleton } from "@/components/ui/skeleton"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import type { AssetDetail } from "@/lib/ipc-contract"
import { usePlaybackFailure } from "./usePlaybackFailure"
import { ExternalOpenNotice } from "./ExternalOpenNotice"
import { MediaPlayer } from "./MediaPlayer"
import { MpvPlayer } from "./MpvPlayer"

/**
 * 내장 플레이어(libmpv 워커)로 재생할 영상 — 쓸 수 있으면 그것으로.
 * 쓸 수 없으면 WebView 로 재생 가능한(`playable`) 영상은 `<video>` 로 대체하고, 아니면 외부 열기 안내.
 * 영상이 아닌 형식(재생 불가 음성)은 외부 열기 안내.
 */
export function BuiltInVideo({ asset, name, playable }: { asset: AssetDetail; name: string; playable: boolean }) {
  const { t } = useTranslation()
  const api = useApi()
  const isVideo = asset.mediaType === "video"
  const { data: available, isPending } = useQuery({
    queryKey: qk.playerAvailable,
    queryFn: () => api.playerAvailable(),
    enabled: isVideo,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  })
  // 외부 열기 안내가 보이는 경우 = 앱 안에서 재생할 수 없음 → 연속 재생은 건너뛴다
  usePlaybackFailure(asset.assetId, !isVideo || (!isPending && available !== true && !playable))
  if (isVideo && isPending) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="m-3 min-h-0 flex-1" />
  }
  // 음성 등 영상이 아닌 형식은 정말 앱에서 재생할 수 없는 형식 — 영상인데 플레이어가 없으면 형식이 아니라 플레이어 문제
  if (!isVideo) return <ExternalOpenNotice assetId={asset.assetId} message={t("viewer.notPlayable")} />
  if (available === true) return <MpvPlayer asset={asset} name={name} />
  if (playable) return <MediaPlayer asset={asset} name={name} />
  return <ExternalOpenNotice assetId={asset.assetId} message={t("errors.player_unavailable")} />
}
