import { useQuery } from "@tanstack/react-query"
import { useCallback, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Skeleton } from "@/components/ui/skeleton"
import { qk } from "@/hooks/keys"
import { useSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import type { AssetDetail } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { BuiltInVideo } from "./BuiltInVideo"
import { MediaPlayer } from "./MediaPlayer"
import { ForceAutoplayContext, useOpenAutoplay } from "./useOpenAutoplay"
import { VideoPoster } from "./VideoPoster"

/**
 * 온라인 영상·음성의 재생 경로 결정.
 * - 영상 + `videoPlayerBuiltIn` ON: 내장 플레이어(libmpv), 없으면 `playable` 영상은 `<video>` 대체
 * - 영상 + OFF, 음성: 재생 가능하면 `MediaPlayer`, 아니면 `BuiltInVideo`(내장 플레이어 / 외부 열기 안내)
 * 설정은 이 컴포넌트가 마운트될 때(자산마다 key) 한 번만 읽는다 — 재생 중 설정을 바꿔도 현재 영상의 엔진은 유지된다.
 * 설정 로딩 중에는 스켈레톤(잘못된 플레이어가 잠깐 뜨지 않게), 로딩 실패 시 기본값(ON)으로 판단한다.
 *
 * 포스터 게이트(`<video>` 경로만): 영상이 정지 상태(설정 "열 때 재생 = 정지")로 열리면 플레이어를 만들지 않고 썸네일 + ▶ (`VideoPoster`)를 보여 주다가,
 * 사용자가 시작하면 그때 플레이어를 자동 재생으로 띄운다(`ForceAutoplayContext`). 아래는 게이트를 건너뛴다(바로 플레이어):
 * 자동 재생 설정·연속 재생의 현재 항목, 시각 지정(`seekMs`)으로 열렸거나 포스터 중 시각 지정 요청이 온 경우, 앱에서 재생할 수 없어 외부 열기 안내가 뜨는 영상. 음성은 게이트 없음.
 * 내장 플레이어 경로에도 게이트가 없다 — 정지 상태로 열어도 첫 프레임이 캔버스에 나오므로 포스터가 필요 없다 (`autoplay=false` 로 열림).
 */
export function ViewerMedia({ asset, name, playable }: { asset: AssetDetail; name: string; playable: boolean }) {
  const { t } = useTranslation()
  const api = useApi()
  const settings = useSettings()
  const isVideo = asset.mediaType === "video"
  const builtInOn = useRef<boolean | null>(null)
  if (builtInOn.current === null && isVideo) {
    if (settings.data) builtInOn.current = settings.data.videoPlayerBuiltIn
    else if (settings.isError) builtInOn.current = true
  }
  const autoplay = useOpenAutoplay(asset.assetId)
  const [started, setStarted] = useState(false)
  const start = useCallback(() => setStarted(true), [])
  const [seekAtOpen] = useState(() => useViewStore.getState().seekMs !== null)
  const seekTick = useViewStore((s) => s.seekTick)
  const [tickAtOpen] = useState(seekTick)
  // 내장 플레이어 사용 가능 여부 — `BuiltInVideo` 와 같은 쿼리. 재생 불가 영상과 설정이 켜진 영상만 묻는다 (어느 경로로 열릴지 알아야 포스터 게이트를 정한다)
  const needsAvailability = isVideo && (!playable || builtInOn.current === true)
  const { data: builtInAvailable, isPending: availabilityPending } = useQuery({
    queryKey: qk.playerAvailable,
    queryFn: () => api.playerAvailable(),
    enabled: needsAvailability,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  })
  if (isVideo && builtInOn.current === null) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="m-3 min-h-0 flex-1" />
  }
  const seekRequested = seekAtOpen || seekTick !== tickAtOpen
  const gateApplies = isVideo && !autoplay && !started && !seekRequested
  // 어느 경로(내장 플레이어 / `<video>`)로 열릴지는 가용성이 정해져야 안다 — 안 그러면 포스터를 건너뛰고 강제 자동 재생으로 열리거나 포스터가 잘못 뜬다
  if (gateApplies && needsAvailability && availabilityPending) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="m-3 min-h-0 flex-1" />
  }
  // 내장 플레이어로 열릴 영상 (설정 ON 이거나 WebView 가 재생하지 못하는 영상) — 정지로 열려도 첫 프레임이 보이므로 포스터 없음
  const builtIn = needsAvailability && builtInAvailable === true
  if (gateApplies && !builtIn && playable) return <VideoPoster asset={asset} name={name} onStart={start} />

  // `<video>`·외부 안내 경로는 여기까지 왔는데 자동 재생이 아니면 사용자가 시작했거나 시각 지정으로 연 것 — 자동 재생으로 연다.
  // 내장 플레이어는 설정을 따른다(정지 설정이면 첫 프레임에서 멈춘 채 열림). 시각 지정으로 열렸으면 재생한다
  const player = (() => {
    if (isVideo && builtInOn.current) return <BuiltInVideo asset={asset} name={name} playable={playable} />
    // 영상 OFF 와 음성은 기존 동작 그대로
    if (!playable) return <BuiltInVideo asset={asset} name={name} playable={false} />
    return <MediaPlayer asset={asset} name={name} />
  })()
  return <ForceAutoplayContext.Provider value={isVideo && (!builtIn || seekRequested)}>{player}</ForceAutoplayContext.Provider>
}
