import { Gauge, Volume2, VolumeX } from "lucide-react"
import { useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Slider } from "@/components/ui/slider"
import { cn } from "@/lib/utils"
import { useViewStore } from "@/stores"
import { FullscreenButton } from "./FullscreenButton"
import { useFocusExclusion } from "./useFocusExclusion"
import type { MediaState } from "./useMediaState"
import { PlayControls, PlayTime, type ContentNav } from "./PlayControls"
import { PlayerTrack } from "./PlayerTrack"
import { RangeControls } from "./RangeControls"
import { SceneControls } from "./SceneControls"
import { SubtitleMenu } from "./SubtitleMenu"
import type { SubtitleUi } from "./useSubtitles"
import type { RangeUi } from "./useRangeController"
import type { SceneUi } from "./useSceneController"
import { VolumeIndicator, useVolumeWheel } from "./useVolumeWheel"
import { PLAYBACK_RATES } from "./viewerMath"

export type { ContentNav } from "./PlayControls"

/** 컨트롤 바가 보여주는 상태와 호출하는 동작 — `<video>`/`<audio>` 요소와 내장 플레이어(IPC) 어댑터가 각각 이 모양을 채운다. */
export interface ControlsBinding {
  state: MediaState
  onTogglePlay: () => void
  onSeek: (seconds: number) => void
  /** 비동기(IPC)이면 Promise 를 돌려준다 — 프레임 이동 길게 누르기는 이전 이동이 끝난 뒤에 다음을 요청한다 */
  onStep: (dir: 1 | -1) => void | Promise<unknown>
  onRate: (rate: number) => void
  /** 0..1 */
  onVolume: (volume: number) => void
  onToggleMute: () => void
}

interface Props extends ControlsBinding {
  /** 메타데이터 로딩 전 길이(ms) — 요소가 아직 길이를 모를 때 쓴다 */
  fallbackDurationMs: number | null
  /** 프레임 이동 버튼 노출 (영상만) */
  frameStep: boolean
  /** 구간 박스·구간 그룹(시작/끝·"구간만"). 없으면 구간 UI 를 그리지 않는다 */
  ranges?: RangeUi
  /** 이전/다음 콘텐츠 버튼. 없으면(이동할 대상 없음) 그리지 않는다 */
  nav?: ContentNav
  /** 음량 변경 직후 잠깐 보일 음량(0..1) — 컨트롤 바 안에 표시 */
  volumeLevel?: number | null
  /** 표시기가 시스템 음량인지 / 시스템 음량 오류 문구 */
  volumeSystem?: boolean
  volumeError?: string | null
  /** 음소거 버튼·음량 슬라이더 위 휠 → (소수) 노치 + Shift(시스템 음량). 스테이지 휠과 같은 누적기(useVolumeNotches)를 넘긴다 */
  onVolumeNotches?: (notches: number, shift: boolean) => void
  /** 장면전환점 눈금·장면 그룹(추출 칩·이전/다음·토글)·우클릭 메뉴 (영상만). 없으면 그리지 않는다 */
  scene?: SceneUi
  /** 자막 버튼(CC)·메뉴 (영상만). 없으면 버튼을 그리지 않는다 */
  subtitles?: SubtitleUi
  /** 시작 전(포스터) 모드 — 재생·이전/다음 콘텐츠만 활성, 탐색·프레임 이동·배속·음량은 비활성 */
  idle?: boolean
}

/**
 * 영상·음성 공통 컨트롤 바 — 위 줄: 탐색 트랙(전체 폭), 아래 줄: 재생 그룹 · 시간 · 구간 · 장면 · 자막/배속/음량. 브라우저 기본 controls 는 쓰지 않는다.
 * 컨테이너 폭이 좁아지면 텍스트 라벨이 아이콘+툴팁으로 줄고, 그룹 단위로 줄바꿈한다.
 */
export function MediaControls({ state, fallbackDurationMs, frameStep, ranges, nav, volumeLevel, volumeSystem, volumeError, onVolumeNotches, subtitles, scene, onTogglePlay, onSeek, onStep, onRate, onVolume, onToggleMute, idle = false }: Props) {
  const root = useRef<HTMLDivElement>(null)
  useFocusExclusion(root)
  const [volumeBox, setVolumeBox] = useState<HTMLDivElement | null>(null)
  useVolumeWheel(volumeBox, (notches, shift) => onVolumeNotches?.(notches, shift))
  const { t } = useTranslation()
  const duration = Number.isFinite(state.duration) && state.duration > 0 ? state.duration : (fallbackDurationMs ?? 0) / 1000
  const shownVolume = state.muted ? 0 : state.volume
  const fullscreen = useViewStore((s) => s.fullscreen)
  return (
    <div
      ref={root}
      data-player-controls=""
      className={cn(
        "@container flex flex-col gap-2 border-t border-border bg-card px-4 py-2",
        // 전체화면: 영상 위 하단 오버레이 — 마우스가 멈추면(부모 pane 의 data-idle) 숨긴다. 영상 영역 크기는 그대로라 렌더 크기가 흔들리지 않는다
        fullscreen && "absolute inset-x-0 bottom-0 z-10 bg-card/90 transition-[opacity,visibility] group-data-[idle=true]/pane:invisible group-data-[idle=true]/pane:opacity-0"
      )}
    >
      <div className="flex min-w-0 flex-col gap-1">
        <PlayerTrack currentSec={state.currentTime} durationSec={duration} onSeek={onSeek} ranges={ranges} scene={scene} disabled={idle} />
      </div>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <div className="flex items-center gap-2">
          <PlayControls paused={state.paused} frameStep={frameStep} nav={nav} stepDisabled={idle} onTogglePlay={onTogglePlay} onStep={onStep} />
          <PlayTime currentSec={state.currentTime} totalSec={duration} />
        </div>
        {ranges && <RangeControls ui={ranges} />}
        {scene && <SceneControls ui={scene} />}
        <div className="relative ml-auto flex items-center gap-1">
          {(volumeLevel !== undefined || volumeError != null) && <VolumeIndicator level={volumeLevel ?? null} system={volumeSystem} error={volumeError} className="top-1/2 right-full mr-2 -translate-y-1/2" />}
          {subtitles && <SubtitleMenu ui={subtitles} />}
          <Select value={String(state.rate)} disabled={idle} onValueChange={(v) => onRate(Number(v))}>
            <SelectTrigger size="sm" aria-label={t("viewer.rate")} title={t("viewer.rate")} className="w-auto gap-1 px-2">
              <Gauge aria-hidden="true" />
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PLAYBACK_RATES.map((r) => (
                <SelectItem key={r} value={String(r)}>
                  {t("viewer.rateOption", { rate: r })}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <div ref={setVolumeBox} className="flex items-center gap-1">
            <Button size="icon-sm" variant="ghost" aria-label={state.muted ? t("viewer.unmute") : t("viewer.mute")} aria-pressed={state.muted} disabled={idle} onClick={onToggleMute}>
              {state.muted || state.volume === 0 ? <VolumeX aria-hidden="true" /> : <Volume2 aria-hidden="true" />}
            </Button>
            <Slider aria-label={t("viewer.volume")} disabled={idle} className="w-20" min={0} max={1} step={0.05} value={[shownVolume]} onValueChange={([v]) => onVolume(v)} />
          </div>
          <FullscreenButton />
        </div>
      </div>
    </div>
  )
}
