import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { usePlayingStepSec } from "@/hooks/usePlayingStepSec"
import { useApi } from "@/lib/api"
import type { AssetDetail } from "@/lib/ipc-contract"
import type { PlayerOpenResult, PlayerStateName } from "@/lib/ipc-contract-player"
import { usePlaybackStore, usePlayerVolumeStore, useViewStore } from "@/stores"
import type { ControlsBinding } from "./MediaControls"

export interface MpvPlayback {
  /** `MediaControls` 에 그대로 넘기는 상태·동작 */
  controls: ControlsBinding
  /** 플레이어가 보고한 상태 */
  playerState: PlayerStateName
  /** `state === "error"` 이벤트의 메시지 */
  error: string | null
  /** 길이(ms) — 이벤트 → 열기 응답 → 메타데이터 순으로 알려진 값 */
  lengthMs: number | null
  /** 워커가 죽어 자동 재시작 중이면 시도 번호 (아니면 null) */
  restarting: { attempt: number; max: number } | null
  /** 자동 재시작이 모두 실패 — 사용자가 다시 시도하거나 외부 프로그램으로 열어야 한다 */
  restartFailed: boolean
  /** 열기 응답의 위치가 상태에 반영됐다 — 이어보기 저장의 문턱 (열기 직후의 0 위치를 저장하지 않게) */
  positioned: boolean
  /** 탐색(seek)이 끝날 때마다 1 씩 증가 — 이어보기 저장의 "탐색 완료" 신호 */
  seekCount: number
}

const noop = () => undefined

/** 위치(ms)를 `[0, 길이]` 로 맞춘다 — 길이를 모르면 아래쪽만 막는다 */
function clampMs(ms: number, length: number | null): number {
  const upper = length !== null && length > 0 ? length : Number.POSITIVE_INFINITY
  return Math.min(Math.max(ms, 0), upper)
}

/**
 * 내장 플레이어(IPC)를 `MediaControls` 가 쓰는 모양으로 바꾸는 어댑터.
 * 위치·상태는 `player://state` 이벤트가 정한다 — 이벤트는 현재 세션(`sessionId`)의 것만 받는다 (응답 전에는 모두).
 * 프레임 이동 뒤의 시각은 로컬에서 계산하지 않고, 워커가 교정한 값이 이벤트로 온다. 탐색(seek)만 슬라이더 반응을 위해 목표 시각을 먼저 보인다.
 * 열기·재시작 응답(`reply`)은 이벤트가 없는 첫 프레임 상태(재생 중/정지, 시각)를 채운다.
 */
export function useMpvPlayback(asset: AssetDetail, openLengthMs: number | null, ready: boolean, sessionId: number | null, reply: PlayerOpenResult | null): MpvPlayback {
  const api = useApi()
  const playingStepSec = usePlayingStepSec()
  const assetId = asset.assetId
  const [playerState, setPlayerState] = useState<PlayerStateName>("opening")
  const [error, setError] = useState<string | null>(null)
  const [restarting, setRestarting] = useState<{ attempt: number; max: number } | null>(null)
  const [restartFailed, setRestartFailed] = useState(false)
  const [eventLengthMs, setEventLengthMs] = useState<number | null>(null)
  const [timeMs, setTimeMs] = useState(0)
  const [positioned, setPositioned] = useState(false)
  const [seekCount, setSeekCount] = useState(0)
  const [rate, setRate] = useState(1)
  // 음량·음소거는 두 재생 경로가 공유하는 스토어 값 — 파일을 바꿔도 유지되고 설정에 저장된다
  const level = usePlayerVolumeStore((s) => s.level)
  const userMuted = usePlayerVolumeStore((s) => s.muted)
  const muted = userMuted || level === 0
  const volume = level / 100
  const stateRef = useRef<PlayerStateName>("opening")
  const sessionRef = useRef(sessionId)
  sessionRef.current = sessionId

  const lengthMs = eventLengthMs ?? openLengthMs ?? asset.durationMs
  const lengthRef = useRef(lengthMs)
  lengthRef.current = lengthMs

  useEffect(() => {
    let disposed = false
    let off: (() => void) | null = null
    void api.onPlayerState((p) => {
      if (p.assetId !== assetId) return
      if (sessionRef.current !== null && p.sessionId < sessionRef.current) return // 이전 세션(닫기 전)의 늦은 이벤트
      stateRef.current = p.state
      setPlayerState(p.state)
      setError(p.state === "error" ? (p.error ?? "") : null)
      setRestarting(p.state === "restarting" ? (p.restart ?? { attempt: 1, max: 1 }) : null)
      setRestartFailed(p.state === "restartFailed")
      if (p.lengthMs !== null) setEventLengthMs(p.lengthMs)
      setTimeMs(p.timeMs)
      // 연속 재생: 끝났거나 오류면 대기열에 알린다 (현재 항목이 아니면 스토어가 무시)
      if (p.state === "ended") usePlaybackStore.getState().reportEnded(assetId)
      else if (p.state === "error") usePlaybackStore.getState().reportFailed(assetId)
    }).then((unlisten) => {
      if (disposed) unlisten()
      else off = unlisten
    }, noop)
    return () => {
      disposed = true
      off?.()
    }
  }, [api, assetId])

  // 열기·재시작 응답 = 첫 프레임이 이미 나온 시점의 상태. 이벤트가 따로 오지 않으므로 여기서 채운다 (재시작 성공이면 알림도 걷는다)
  useEffect(() => {
    if (!reply) return
    stateRef.current = reply.state
    setPlayerState(reply.state)
    setTimeMs(reply.timeMs)
    setPositioned(true)
    setError(null)
    setRestarting(null)
    setRestartFailed(false)
  }, [reply])

  /** 실패는 삼키고, 끝나는 시점을 알 수 있게 Promise 를 돌려준다 (프레임 이동 길게 누르기가 이전 이동이 끝나길 기다린다) */
  const run = useCallback((p: Promise<unknown>) => p.catch(noop), [])

  const seekMsTo = useCallback(
    (target: number) => {
      const ms = Math.round(clampMs(target, lengthRef.current))
      setTimeMs(ms)
      return run(api.playerSeek(ms)).then(() => setSeekCount((n) => n + 1))
    },
    [api, run]
  )

  // 열기가 끝나면 UI 의 초기 속도를 플레이어에 맞춘다 (플레이어는 자산을 바꿔도 하나를 재사용한다)
  useEffect(() => {
    if (ready) run(api.playerSetRate(1))
  }, [api, ready, run])

  // 저장된 음량·음소거를 열기 직후와 변경(슬라이더·휠·키·M·설정 로드)마다 플레이어에 맞춘다. 음량 0 은 음소거로 보낸다
  useEffect(() => {
    if (ready) run(api.playerSetVolume(level, muted))
  }, [api, ready, level, muted, run])

  // AI 자막 클릭 등 외부 seek 요청 — 열기가 끝난 뒤 적용한다
  const seekRequest = useViewStore((s) => s.seekMs)
  const seekTick = useViewStore((s) => s.seekTick)
  useEffect(() => {
    if (ready && seekRequest !== null) seekMsTo(seekRequest)
  }, [ready, seekRequest, seekTick, seekMsTo])

  const binding = useMemo<ControlsBinding>(
    () => ({
      state: {
        currentTime: timeMs / 1000,
        duration: lengthMs !== null && lengthMs > 0 ? lengthMs / 1000 : Number.NaN,
        paused: playerState !== "playing",
        rate,
        volume,
        muted,
      },
      onTogglePlay: () => {
        if (ready) run(api.playerControl("toggle"))
      },
      onSeek: (seconds) => {
        if (ready) seekMsTo(seconds * 1000)
      },
      onStep: (dir) => {
        if (!ready) return
        // 재생 중이면 초 단위 seek 으로 재생을 유지한다 (stepForward/stepBack 은 워커가 일시정지한다)
        if (stateRef.current === "playing") return seekMsTo(timeMs + dir * playingStepSec * 1000)
        // 정지 중: 한 프레임 이동 — 이동 뒤 시각은 워커가 교정해 state 이벤트로 보낸다 (로컬 계산 없음)
        return run(api.playerControl(dir > 0 ? "stepForward" : "stepBack"))
      },
      onRate: (r) => {
        setRate(r)
        if (ready) run(api.playerSetRate(r))
      },
      onVolume: (v) => {
        usePlayerVolumeStore.getState().setLevel(Math.round(v * 100))
      },
      onToggleMute: () => {
        usePlayerVolumeStore.getState().setMuted(!muted)
      },
    }),
    [api, lengthMs, muted, playerState, playingStepSec, rate, ready, run, seekMsTo, timeMs, volume]
  )

  return { controls: binding, playerState, error, lengthMs, restarting, restartFailed, positioned, seekCount }
}
