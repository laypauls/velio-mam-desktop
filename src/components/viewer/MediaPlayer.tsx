import { FileAudio } from "lucide-react"
import { useCallback, useEffect, useState } from "react"
import { usePlayingStepSec } from "@/hooks/usePlayingStepSec"
import { usePreviewSeek } from "@/hooks/usePreviewSeek"
import { useApi } from "@/lib/api"
import type { AssetDetail } from "@/lib/ipc-contract"
import { usePlaybackStore, usePlayerVolumeStore } from "@/stores"
import { MediaControls, type ControlsBinding } from "./MediaControls"
import { PlayFeedbackOverlay } from "./PlayFeedbackOverlay"
import { resumeRestartTarget } from "./resumeRanges"
import { ResumeNotice } from "./ResumeNotice"
import { useMediaState } from "./useMediaState"
import { nextVolume } from "./playerKeys"
import { useHtmlResume, useSeekedCount } from "./useHtmlResume"
import { useHtmlSubtitle } from "./useHtmlSubtitle"
import { useMetadataLoaded } from "./useMetadataLoaded"
import { useContentNav } from "./useContentNav"
import { useOpenAutoplay } from "./useOpenAutoplay"
import { movedBy, usePlayFeedback } from "./usePlayFeedback"
import { usePlayerKeys } from "./usePlayerKeys"
import { useResumeEnabled, useResumeRestore } from "./useResume"
import { useResumeSave } from "./useResumeSave"
import { useRangeController } from "./useRangeController"
import { useRangePlayback } from "./useRangePlayback"
import { useSceneController } from "./useSceneController"
import { useSubtitles } from "./useSubtitles"
import { useSystemVolume } from "./useSystemVolume"
import { useToggleFullscreen } from "./useToggleFullscreen"
import { VolumeIndicator, useVolumeIndicator, useVolumeWheel } from "./useVolumeWheel"
import { useVolumeNotches } from "./wheelNotches"
import { ViewerContextMenu } from "./ViewerContextMenu"
import { clampTime, stepFrame } from "./viewerMath"

/** 영상·음성 재생 — `<video>`/`<audio>` + 커스텀 컨트롤 바. 단축키는 usePlayerKeys 가 window 에서 처리한다. */
export function MediaPlayer({ asset, name }: { asset: AssetDetail; name: string }) {
  const api = useApi()
  const [el, setEl] = useState<HTMLMediaElement | null>(null)
  const seekRef = usePreviewSeek()
  const mediaRef = useCallback(
    (node: HTMLMediaElement | null) => {
      setEl(node)
      seekRef(node)
    },
    [seekRef]
  )
  const [stage, setStage] = useState<HTMLDivElement | null>(null)
  const indicator = useVolumeIndicator()
  const { flash } = indicator
  const state = useMediaState(el)
  const isVideo = asset.mediaType === "video"
  const src = api.mediaUrl(asset.assetId)
  // 외부 자막 — 영상만 (음성은 조회하지 않는다)
  // 영상을 실제로 열 수 있다는 것(메타데이터 로드, 오류 없음)이 확인된 뒤에만 자막을 찾는다
  const opened = useMetadataLoaded(el, src)
  const subtitles = useSubtitles(asset.assetId, isVideo && opened, isVideo && !opened)
  useHtmlSubtitle(isVideo ? el : null, asset.assetId, subtitles.effectiveId)
  // 연속 재생의 현재 항목이거나 설정 "열 때 자동 재생" 이 켜져 있으면 열리면 자동 재생하고, 끝나면/오류가 나면 대기열에 알린다
  const autoplay = useOpenAutoplay(asset.assetId)
  const onEnd = () => usePlaybackStore.getState().reportEnded(asset.assetId)
  const onError = () => usePlaybackStore.getState().reportFailed(asset.assetId)
  useEffect(() => {
    // 자동 재생이 막히면(NotAllowedError) 건너뛰지 않고 일시정지 상태로 둔다
    if (el && autoplay) void Promise.resolve(el.play()).catch(() => undefined)
  }, [el, autoplay])

  const togglePlay = () => {
    if (!el) return
    if (el.paused) void Promise.resolve(el.play()).catch(() => undefined)
    else el.pause()
  }
  const seekTo = (seconds: number) => {
    if (el) el.currentTime = clampTime(seconds, el.duration)
  }
  const playingStepSec = usePlayingStepSec()
  // 재생 중에는 초 단위로 이동하고 재생을 유지한다. 정지 중에는 1프레임 (키보드 , . 도 같은 경로)
  const step = (dir: 1 | -1) => {
    if (!el) return
    if (!el.paused) {
      seekTo(el.currentTime + dir * playingStepSec)
      return
    }
    el.pause()
    el.currentTime = stepFrame(el.currentTime, asset.fps, dir, el.duration)
  }
  // 사용자 입력 경로만 중앙 표시를 띄운다 (자동 재생·구간 끝 정지·이동 seek 은 거치지 않는다)
  const feedback = usePlayFeedback()
  const userTogglePlay = () => {
    if (el) feedback.flashToggle(!el.paused)
    togglePlay()
  }
  const userSeekBy = (delta: number) => {
    if (!el) return
    feedback.flashSeek(movedBy(el.currentTime, delta, el.duration))
    seekTo(el.currentTime + delta)
  }
  const userStep = (dir: 1 | -1) => {
    if (el && !el.paused) feedback.flashSeek(movedBy(el.currentTime, dir * playingStepSec, el.duration)) // 정지 중 1프레임 이동은 표시하지 않는다
    step(dir)
  }
  const toggleMute = () => {
    if (!el) return
    el.muted = !el.muted
    usePlayerVolumeStore.getState().setMuted(el.muted)
  }

  const setVolume = (v: number) => {
    if (!el) return
    el.volume = v
    el.muted = v === 0
    usePlayerVolumeStore.getState().setLevel(Math.round(v * 100))
  }
  // 저장된 음량·음소거를 열 때(요소가 생길 때) 적용하고, 설정 로드 등으로 값이 바뀌어도 맞춘다. 음량 0 은 음소거로 둔다
  const savedLevel = usePlayerVolumeStore((s) => s.level)
  const savedMuted = usePlayerVolumeStore((s) => s.muted)
  useEffect(() => {
    if (!el) return
    if (Math.round(el.volume * 100) !== savedLevel) el.volume = savedLevel / 100
    el.muted = savedMuted || savedLevel === 0
  }, [el, savedLevel, savedMuted])
  const volumeBy = (delta: number) => {
    if (!el) return
    const v = nextVolume(el.muted ? 0 : el.volume, delta)
    setVolume(v)
    flash(v)
  }
  const systemVolumeBy = useSystemVolume(indicator)
  const controls: ControlsBinding = {
    state,
    onTogglePlay: userTogglePlay,
    onSeek: seekTo,
    onStep: userStep,
    onRate: (rate) => {
      if (el) el.playbackRate = rate
    },
    onVolume: setVolume,
    onToggleMute: toggleMute,
  }
  const rangeCtl = useRangeController(asset, controls)
  // 이어보기 — 메타데이터가 로드되면 기록 위치로 이동(구간만 재생이면 구간 안으로 보정). 저장은 복원이 끝난 뒤부터
  const resume = useResumeRestore(asset.assetId)
  const htmlResume = useHtmlResume(el, resume, rangeCtl.activeRanges)
  const resumeEnabled = useResumeEnabled()
  const seekedCount = useSeekedCount(el)
  useResumeSave({
    assetId: asset.assetId,
    enabled: resumeEnabled,
    armed: htmlResume.armed && (!resume.lookupFailed || !state.paused),
    playing: !state.paused,
    positionMs: el !== null && htmlResume.armed ? el.currentTime * 1000 : 0,
    durationMs: Number.isFinite(state.duration) && state.duration > 0 ? state.duration * 1000 : asset.durationMs,
    seekTick: seekedCount,
  })
  // 장면전환점 — 영상만 (요소가 현재 시각을 정확히 알므로 키 이동·추가 기준에 직접 읽는다)
  const sceneCtl = useSceneController(asset, controls, () => (el ? el.currentTime * 1000 : 0))
  // 구간만 재생: 마지막 구간 끝에서 멈추면 네이티브 ended 가 오지 않으므로 연속 재생에 직접 알린다
  useRangePlayback(el, rangeCtl.activeRanges, { isVideo, fps: asset.fps, onEnd })
  const contentNav = useContentNav()
  const toggleFullscreen = useToggleFullscreen()
  usePlayerKeys({
    ready: el !== null,
    contentNav: contentNav?.standalone ? contentNav : undefined,
    ranges: rangeCtl.keys,
    scenes: sceneCtl.keys,
    isVideo,
    togglePlay: userTogglePlay,
    seekBy: userSeekBy,
    step: userStep,
    toggleMute,
    volumeBy,
    systemVolumeBy,
    toggleFullscreen,
  })
  const wheelNotches = useVolumeNotches(volumeBy, systemVolumeBy) // 스테이지와 컨트롤 바 음량 영역의 휠이 같은 누적기를 쓴다
  useVolumeWheel(stage, wheelNotches)

  return (
    <div tabIndex={-1} className="relative flex min-h-0 flex-1 flex-col outline-none">
      <ViewerContextMenu asset={asset}>
        <div ref={setStage} className="relative flex min-h-0 flex-1 items-center justify-center bg-viewer-stage" onClick={userTogglePlay}>
          <PlayFeedbackOverlay toggle={feedback.toggle} seek={feedback.seek} />
          <ResumeNotice restoredMs={htmlResume.restoredMs} onRestart={() => seekTo(resumeRestartTarget(rangeCtl.activeRanges) / 1000)} />
          <VolumeIndicator level={indicator.level} system={indicator.system} error={indicator.error} />
          {isVideo ? (
            <video ref={mediaRef} src={src} aria-label={name} preload="metadata" onEnded={onEnd} onError={onError} className="size-full object-contain" />
          ) : (
            <>
              <audio ref={mediaRef} src={src} aria-label={name} preload="metadata" onEnded={onEnd} onError={onError} />
              <FileAudio aria-hidden="true" className="size-16 text-media-audio" />
            </>
          )}
        </div>
      </ViewerContextMenu>
      <MediaControls {...controls} fallbackDurationMs={asset.durationMs} frameStep={isVideo} nav={contentNav} ranges={rangeCtl.ui} scene={sceneCtl.ui} onVolumeNotches={wheelNotches} subtitles={isVideo ? subtitles : undefined} />
    </div>
  )
}
