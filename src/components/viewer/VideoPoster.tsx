import { Play } from "lucide-react"
import { useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useApi } from "@/lib/api"
import type { AssetDetail } from "@/lib/ipc-contract"
import { usePlayerVolumeStore } from "@/stores"
import { MediaControls, type ControlsBinding } from "./MediaControls"
import { useContentNav } from "./useContentNav"
import { usePosterStart } from "./usePosterStart"

const noop = () => undefined

/**
 * 정지 상태로 여는 영상의 시작 전 화면 — 썸네일 + 가운데 ▶. 플레이어(`player_open`·`<video>`)는 사용자가 시작할 때까지 만들지 않는다.
 * 컨트롤 바는 본 플레이어와 같은 레이아웃(길이는 DB 값)이고 재생·이전/다음 콘텐츠만 활성이다. 썸네일 로딩 실패 시 배경 + ▶ 만 남긴다.
 */
export function VideoPoster({ asset, name, onStart }: { asset: AssetDetail; name: string; onStart: () => void }) {
  const { t } = useTranslation()
  const api = useApi()
  const rootRef = useRef<HTMLDivElement>(null)
  const [thumbFailed, setThumbFailed] = useState(false)
  const contentNav = useContentNav()
  const level = usePlayerVolumeStore((s) => s.level)
  const muted = usePlayerVolumeStore((s) => s.muted)
  usePosterStart(rootRef, onStart)
  const controls: ControlsBinding = {
    state: { currentTime: 0, duration: Number.NaN, paused: true, rate: 1, volume: level / 100, muted },
    onTogglePlay: onStart,
    onSeek: noop,
    onStep: noop,
    onRate: noop,
    onVolume: noop,
    onToggleMute: noop,
  }
  return (
    <div ref={rootRef} tabIndex={-1} data-testid="video-poster" className="flex min-h-0 flex-1 flex-col outline-none">
      <div onClick={onStart} className="relative flex min-h-0 flex-1 cursor-pointer items-center justify-center bg-viewer-stage">
        {!thumbFailed && (
          <img src={api.thumbUrl(asset.assetId, 1024)} alt={name} data-testid="poster-thumb" onError={() => setThumbFailed(true)} className="absolute inset-0 size-full object-contain" />
        )}
        <Button size="icon-lg" variant="default" aria-label={t("viewer.play")} title={t("viewer.play")} onClick={(e) => {
            e.stopPropagation() // 무대 클릭과 겹쳐 두 번 시작되지 않게
            onStart()
          }} className="relative size-16 rounded-full">
          <Play aria-hidden="true" className="size-8" />
        </Button>
      </div>
      <MediaControls {...controls} idle fallbackDurationMs={asset.durationMs} frameStep nav={contentNav} />
    </div>
  )
}
