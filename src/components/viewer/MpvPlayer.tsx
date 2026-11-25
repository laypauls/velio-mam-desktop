import { useCallback, useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { usePlayingStepSec } from "@/hooks/usePlayingStepSec"
import { useApi } from "@/lib/api"
import type { AssetDetail, PlayRange } from "@/lib/ipc-contract"
import { errorKey } from "@/lib/errors"
import { ExternalOpenNotice } from "./ExternalOpenNotice"
import { MediaControls, type ControlsBinding } from "./MediaControls"
import { PlayFeedbackOverlay } from "./PlayFeedbackOverlay"
import { PlayerRestartNotice } from "./PlayerRestartNotice"
import { resumeRestartTarget } from "./resumeRanges"
import { ResumeNotice } from "./ResumeNotice"
import { nextVolume } from "./playerKeys"
import { FALLBACK_ASPECT, useCanvasSize, useRenderSizeReport } from "./useCanvasSize"
import { useContentNav } from "./useContentNav"
import { useFrameBuffers } from "./useFrameBuffers"
import { useMpvPlayback } from "./useMpvPlayback"
import { useMpvRanges, useMpvSession, useMpvSubtitle } from "./useMpvSession"
import { useOpenAutoplay } from "./useOpenAutoplay"
import { useResumeEnabled, useResumeRestore } from "./useResume"
import { useResumeSave } from "./useResumeSave"
import { movedBy, usePlayFeedback } from "./usePlayFeedback"
import { usePlayerKeys } from "./usePlayerKeys"
import { usePlaybackFailure } from "./usePlaybackFailure"
import { useRangeController } from "./useRangeController"
import { useRenderActive } from "./useRenderActive"
import { useSceneController } from "./useSceneController"
import { useSubtitles } from "./useSubtitles"
import { useSystemVolume } from "./useSystemVolume"
import { useToggleFullscreen } from "./useToggleFullscreen"
import { useVolumeIndicator, useVolumeWheel } from "./useVolumeWheel"
import { useWebglRenderer } from "./useWebglRenderer"
import { useVolumeNotches } from "./wheelNotches"

/** 자산의 폭·높이로 정한 영상 비율(폭/높이). 모르면 null */
function assetAspect(asset: AssetDetail): number | null {
  const { width, height } = asset
  return width !== null && height !== null && width > 0 && height > 0 ? width / height : null
}

/**
 * 내장 플레이어(libmpv 워커) — 워커가 렌더한 프레임을 SharedBuffer 로 받아 WebGL2 `<canvas>` 에 그린다.
 * 캔버스는 일반 DOM 요소라 메뉴·대화상자·toast 가 영상 위에 그대로 열린다 (스냅샷·창 숨김 없음).
 * 무대(`bg-viewer-stage`)가 레터박스 배경이고, 캔버스는 무대를 채운 채 영상 비율로 "contain" 된다(`object-contain`).
 * 호스트에 알리는 렌더 크기는 그 contain 영역의 장치 px 다.
 */
export function MpvPlayer({ asset, name }: { asset: AssetDetail; name: string }) {
  const { t } = useTranslation()
  const api = useApi()
  // 요소 state — 요소가 준비된 뒤 효과가 실행되도록 (ref 객체는 변경을 알리지 않는다)
  const [stageEl, setStageEl] = useState<HTMLDivElement | null>(null)
  const [canvasEl, setCanvasEl] = useState<HTMLCanvasElement | null>(null)
  // 열 때 자동 재생 여부 — 연속 재생 항목은 항상 true, 그 밖에는 설정 (열기 시점 값만 쓴다)
  const autoplay = useOpenAutoplay(asset.assetId)
  // 열기에 실어 보낼 구간 — 구간 컨트롤러는 세션 뒤에 만들어지므로 ref 로 넘기고 열기 실행 시점에 읽는다
  const openRanges = useRef<readonly PlayRange[] | null | undefined>(undefined)
  // 이어보기 — 기록을 알아야 열기에 시작 위치를 실을 수 있으므로 조회가 끝날 때까지 열기를 미룬다 (열기 실행 시점에 ref 를 읽는다)
  const resume = useResumeRestore(asset.assetId)
  const resumeStart = useRef<number | null>(null)
  resumeStart.current = resume.recordMs

  // 프레임 리스너는 열기보다 먼저 설치돼야 한다 — 세션 훅보다 앞에서 호출
  const frames = useFrameBuffers()
  // 비율은 자산 정보에서 정한다. 자산 정보가 없을 때만 첫 프레임의 비율을 한 번 고정한다 — 프레임 크기를 계속 따라가면 워커가 요청보다 조금 좁게 그린 프레임이
  // 다시 비율이 되어 렌더 크기가 한 단계씩 줄어든다(수렴 드리프트)
  const frameAspect = useRef<number | null>(null)
  if (frameAspect.current === null && frames.frameSize) frameAspect.current = frames.frameSize.w / frames.frameSize.h
  const metrics = useCanvasSize(stageEl, assetAspect(asset) ?? frameAspect.current ?? FALLBACK_ASPECT)
  const glStatus = useWebglRenderer(canvasEl, frames.source, useCallback(() => void api.playerSetRenderActive(true).catch(() => undefined), [api]))
  const glOk = glStatus === "ok"
  const session = useMpvSession(asset.assetId, glOk && resume.resolved ? metrics.size : null, autoplay, openRanges, resumeStart)
  const ready = session.phase === "ready"
  const playback = useMpvPlayback(asset, session.lengthMs, ready, session.sessionId, session.reply)
  const failed = !glOk || session.phase === "failed" || playback.error !== null
  const live = ready && !failed
  usePlaybackFailure(asset.assetId, failed)
  useRenderSizeReport(metrics, live, session.openSize)
  const { setRendering } = frames
  const rendering = useRenderActive(live, metrics.visible)
  useEffect(() => setRendering(rendering), [setRendering, rendering])

  const { controls: base } = playback
  // 사용자 입력 경로(무대 클릭·키·컨트롤 바 버튼)만 중앙 표시를 띄운다 — 워커 상태 전이(자동 재생·구간 끝 정지·재시작 복원)로는 띄우지 않는다
  const feedback = usePlayFeedback()
  const playingStepSec = usePlayingStepSec()
  const { state } = base
  const controls: ControlsBinding = {
    ...base,
    onTogglePlay: () => {
      feedback.flashToggle(!state.paused) // 워커 상태는 응답 뒤 갱신되므로 입력 시점의 값을 뒤집는다
      base.onTogglePlay()
    },
    onStep: (dir) => {
      if (!state.paused) feedback.flashSeek(movedBy(state.currentTime, dir * playingStepSec, state.duration)) // 정지 중 1프레임 이동은 표시하지 않는다
      base.onStep(dir)
    },
  }
  const seekBy = (delta: number) => {
    feedback.flashSeek(movedBy(state.currentTime, delta, state.duration))
    base.onSeek(state.currentTime + delta)
  }
  const rangeCtl = useRangeController(asset, controls)
  // 이어보기 저장 — 열기 응답의 위치가 반영된 뒤부터 (기록 조회가 실패했으면 사용자가 재생한 뒤부터)
  const resumeEnabled = useResumeEnabled()
  useResumeSave({
    assetId: asset.assetId,
    enabled: resumeEnabled,
    armed: live && playback.positioned && (!resume.lookupFailed || !state.paused),
    playing: !state.paused,
    positionMs: state.currentTime * 1000,
    durationMs: playback.lengthMs,
    seekTick: playback.seekCount,
  })
  const sceneCtl = useSceneController(asset, controls)
  // 플레이어는 열 때마다 구간을 잊는다 — 열기 성공 뒤와 구간·토글 변경 때마다 보낸다
  openRanges.current = rangeCtl.activeRanges
  useMpvRanges(asset.assetId, ready, rangeCtl.activeRanges, session.openedKey)
  // 외부 자막 — 플레이어는 열 때마다 자막을 끄고 시작하므로 열기 성공 뒤 고른 트랙을 보낸다 (워커가 프레임에 직접 그린다)
  const subtitles = useSubtitles(asset.assetId, ready && !failed, !ready && !failed)
  useMpvSubtitle(asset.assetId, ready, subtitles.effectiveId)

  // 음량 변경 — 연속으로 빠르게 들어오는 휠 입력이 렌더 전의 낡은 값 위에 쌓이지 않도록 마지막으로 정한 값을 ref 로 이어간다
  const indicator = useVolumeIndicator()
  const { flash } = indicator
  const systemVolumeBy = useSystemVolume(indicator)
  const shownVolume = controls.state.muted ? 0 : controls.state.volume
  const volumeRef = useRef(shownVolume)
  volumeRef.current = shownVolume
  const volumeBy = (delta: number) => {
    const v = nextVolume(volumeRef.current, delta)
    volumeRef.current = v
    controls.onVolume(v)
    flash(v)
  }
  const wheelNotches = useVolumeNotches(volumeBy, systemVolumeBy) // 휠 누적기 (Shift = 시스템 음량)
  useVolumeWheel(stageEl, wheelNotches)

  // 열린 뒤에 플레이어가 오류를 알려 오면 안내가 보이도록 플레이어를 닫는다
  useEffect(() => {
    if (playback.error !== null) void api.playerClose().catch(() => undefined)
  }, [api, playback.error])

  // 훅이므로 조기 반환(failed) 앞에서 호출한다 — 실패하면 ready 를 false 로 넘겨 키를 처리하지 않는다
  const contentNav = useContentNav()
  const toggleFullscreen = useToggleFullscreen()
  usePlayerKeys({
    ready: live,
    contentNav: contentNav?.standalone ? contentNav : undefined,
    ranges: rangeCtl.keys,
    scenes: sceneCtl.keys,
    isVideo: true,
    togglePlay: controls.onTogglePlay,
    seekBy,
    step: controls.onStep,
    toggleMute: controls.onToggleMute,
    volumeBy,
    systemVolumeBy,
    toggleFullscreen,
  })

  if (failed) {
    const unsupported = !glOk && session.error === null
    const reason = unsupported ? t("errors.player_unavailable") : session.error !== null ? t(errorKey(session.error)) : [t("viewer.builtInFailed"), playback.error].filter(Boolean).join(" ")
    return <ExternalOpenNotice assetId={asset.assetId} message={reason} isError />
  }

  const restartStatus = playback.restartFailed ? "failed" : playback.restarting ? "restarting" : null
  return (
    <div tabIndex={-1} className="relative flex min-h-0 flex-1 flex-col outline-none">
      {/* 안내는 role="img" 무대의 자식이면 보조 기술에 노출되지 않는다 — 무대와 형제로 두고 위에 겹친다 */}
      <div className="relative flex min-h-0 flex-1 flex-col">
        <div ref={setStageEl} role="img" aria-label={name} aria-busy={!ready} onClick={controls.onTogglePlay} className="relative min-h-0 flex-1 overflow-hidden bg-viewer-stage">
          <canvas ref={setCanvasEl} aria-hidden="true" className="absolute inset-0 size-full object-contain" />
          <PlayFeedbackOverlay toggle={feedback.toggle} seek={feedback.seek} />
        </div>
        <ResumeNotice restoredMs={session.startMs} onRestart={() => base.onSeek(resumeRestartTarget(rangeCtl.activeRanges) / 1000)} />
        {restartStatus !== null && <PlayerRestartNotice assetId={asset.assetId} status={restartStatus} attempt={playback.restarting} onRetry={session.restart} />}
      </div>
      <MediaControls {...controls} fallbackDurationMs={asset.durationMs} frameStep nav={contentNav} ranges={rangeCtl.ui} scene={sceneCtl.ui} volumeLevel={indicator.level} volumeSystem={indicator.system} volumeError={indicator.error} onVolumeNotches={wheelNotches} subtitles={subtitles} />
    </div>
  )
}
