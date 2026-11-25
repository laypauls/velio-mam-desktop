import { useCallback, useEffect, useRef, useState, type RefObject } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import type { CommandError, PlayRange } from "@/lib/ipc-contract"
import type { CanvasSize, PlayerOpenResult } from "@/lib/ipc-contract-player"
import { fitResumeToRanges } from "./resumeRanges"

/**
 * open/close 는 Rust 쪽 플레이어가 하나뿐이라 순서가 바뀌면 새로 연 세션을 이전 세션의 close 가 닫아 버린다
 * (자산 전환·StrictMode 재마운트) — 앞선 작업이 끝난 뒤에 실행되도록 직렬화한다.
 */
let chain: Promise<unknown> = Promise.resolve()
function enqueue<T>(job: () => Promise<T>): Promise<T> {
  const run = chain.then(job, job)
  chain = run.catch(() => undefined)
  return run
}

/** 구간 → 와이어 값(정수 ms). `null`/`undefined`/빈 배열 = 끔(null) */
export function toWireRanges(ranges: readonly PlayRange[] | null | undefined): PlayRange[] | null {
  if (!ranges || ranges.length === 0) return null
  return ranges.map((r) => ({ startMs: Math.round(r.startMs), endMs: Math.round(r.endMs) }))
}
const rangesKey = (wire: PlayRange[] | null) => JSON.stringify(wire)

export type SessionPhase = "opening" | "ready" | "failed"

export interface MpvSession {
  phase: SessionPhase
  /** `phase === "failed"` 일 때 `player_open` 의 오류 */
  error: CommandError | null
  lengthMs: number | null
  /** 가장 최근 열기·재시작 응답의 세션 번호 — 이벤트 거르기용 (응답 전 null) */
  sessionId: number | null
  /** 가장 최근 열기·재시작 응답 — 재생 상태(첫 프레임에서 재생 중인지)의 초기값 */
  reply: PlayerOpenResult | null
  /** 열기에 실어 보낸 캔버스 크기 — 이후 크기 변경 보고의 기준 */
  openSize: CanvasSize | null
  /** `player_open` 에 실어 보낸 구간의 키 — 플레이어가 이미 아는 구간이라 열기 직후 같은 값을 다시 보내지 않는다 ("null" = 안 보냄) */
  openedKey: string
  /** `player_open` 에 실어 보낸 이어보기 시작 위치(ms, 구간 보정 뒤) — 이어 재생 알림의 기준. 보내지 않았으면 null */
  startMs: number | null
  /** 워커 재시작 실패 뒤 다시 시도 — 실패하면 reject (호출부가 알린다) */
  restart: () => Promise<void>
}

type Snapshot = Omit<MpvSession, "restart">
const OPENING: Snapshot = { phase: "opening", error: null, lengthMs: null, sessionId: null, reply: null, openSize: null, openedKey: "null", startMs: null }

function toCommandError(e: unknown): CommandError {
  if (typeof e === "object" && e !== null && "code" in e && "message" in e) return e as CommandError
  return { code: "open_failed", message: typeof e === "string" ? e : "open failed" }
}

/**
 * 캔버스가 처음 0 이 아닌 크기를 갖게 되면(`canvas !== null`) 플레이어를 열고, 언마운트·자산 변경 시 닫는다.
 * `openRanges` 는 열기 실행 시점에 읽는 구간(`activeRanges`) — 배열이면 `player_open` 에 함께 보낸다 (null/undefined = 안 보냄).
 * `autoplay` 는 열 때의 값만 쓴다. false 면 첫 프레임에서 정지한 채 열리고 응답과 함께 `ready` — 첫 프레임이 이미 캔버스에 있다.
 * 열기에 보내는 캔버스 크기는 대기열 실행 시점의 최신 값(`canvasRef`)이다.
 * `resumeStart` 는 이어보기 시작 위치(ms) — 열기 실행 시점에 읽고, 구간만 재생이면 D5 규칙으로 구간 안으로 보정해 `player_open` 의 `startMs` 로 보낸다 (null/undefined = 안 보냄).
 */
export function useMpvSession(
  assetId: number,
  canvas: CanvasSize | null,
  autoplay: boolean,
  openRanges?: RefObject<readonly PlayRange[] | null | undefined>,
  resumeStart?: RefObject<number | null | undefined>
): MpvSession {
  const api = useApi()
  const autoplayRef = useRef(autoplay)
  autoplayRef.current = autoplay
  const canvasRef = useRef(canvas)
  canvasRef.current = canvas
  const hasCanvas = canvas !== null
  const [session, setSession] = useState<Snapshot>(OPENING)
  useEffect(() => {
    if (!hasCanvas) return
    let cancelled = false
    setSession(OPENING)
    let openedKey = "null"
    let openSize: CanvasSize | null = null
    let openStart: number | null = null
    enqueue(() => {
      const size = canvasRef.current
      if (size === null) return Promise.reject({ code: "open_failed", message: "no canvas size" })
      openSize = size
      // 구간만 재생이 켜져 있고 구간을 이미 받았다면 열기와 함께 보낸다 — 열기 직후 0 부터 재생되는 구간이 없다. 순서 대기 뒤 실행 시점의 값을 쓴다
      const wire = toWireRanges(openRanges?.current)
      const rawStart = resumeStart?.current
      openStart = rawStart === undefined || rawStart === null ? null : Math.round(fitResumeToRanges(rawStart, wire))
      if (wire === null) return openStart === null ? api.playerOpen(assetId, size, autoplayRef.current) : api.playerOpen(assetId, size, autoplayRef.current, undefined, openStart)
      openedKey = rangesKey(wire)
      return openStart === null ? api.playerOpen(assetId, size, autoplayRef.current, wire) : api.playerOpen(assetId, size, autoplayRef.current, wire, openStart)
    }).then(
      (r) => {
        if (!cancelled) setSession({ phase: "ready", error: null, lengthMs: r.lengthMs, sessionId: r.sessionId, reply: r, openSize, openedKey, startMs: openStart })
      },
      (e: unknown) => {
        if (!cancelled) setSession({ ...OPENING, phase: "failed", error: toCommandError(e) })
      }
    )
    return () => {
      cancelled = true
      void enqueue(() => api.playerClose()).catch(() => undefined)
    }
  }, [api, assetId, hasCanvas, openRanges, resumeStart])

  const restart = useCallback(async () => {
    const r = await enqueue(() => api.playerRestart())
    setSession((prev) => ({ ...prev, phase: "ready", error: null, lengthMs: r.lengthMs ?? prev.lengthMs, sessionId: r.sessionId, reply: r }))
  }, [api])
  return { ...session, restart }
}

/**
 * 구간만 재생 — 열기가 끝난 뒤(및 구간·토글이 바뀔 때마다) 플레이어에 구간을 보낸다. 플레이어는 열 때마다 구간을 잊으므로 열기 성공 뒤 반드시 다시 보낸다.
 * `ranges`: `null` = 끔(열린 직후 상태와 같아 보내지 않음, 켰다 끄면 `null` 전송), `undefined` = 아직 모름(보내지 않음).
 * open/close 와 같은 직렬 대기열에 넣어 순서가 뒤집히지 않게 한다. 실패해도 재생 자체는 영향이 없다.
 */
export function useMpvRanges(assetId: number, ready: boolean, ranges: readonly PlayRange[] | null | undefined, openedKey: string = "null"): void {
  const api = useApi()
  // 플레이어가 알고 있는 마지막 값 — null = 아직 열리지 않음, 방금 열렸으면 열기에 실어 보낸 값("끔"이면 "null")
  const sent = useRef<string | null>(null)
  const key = ranges === undefined ? undefined : rangesKey(toWireRanges(ranges))
  useEffect(() => {
    if (!ready) {
      sent.current = null
      return
    }
    if (sent.current === null) sent.current = openedKey
    if (key === undefined || key === sent.current) return
    sent.current = key
    void enqueue(() => api.playerSetRanges(assetId, toWireRanges(ranges))).catch(() => undefined)
  }, [api, assetId, ready, key, openedKey])
}

/**
 * 자막 선택 — 열기가 끝난 뒤 처음(고른 트랙이 있을 때)과 바뀔 때마다 플레이어에 보낸다. 플레이어는 열 때마다 자막을 끈 상태로 시작하므로
 * 열기 성공 뒤마다 다시 보낸다(`useMpvRanges` 와 같은 "sent" 패턴). `trackId`: `null` = 끔, `undefined` = 아직 모름(보내지 않음).
 * open/close 와 같은 직렬 대기열에 넣는다. 실패해도 재생은 계속되고 토스트는 실패마다 1회.
 */
export function useMpvSubtitle(assetId: number, ready: boolean, trackId: string | null | undefined): void {
  const api = useApi()
  const { t } = useTranslation()
  // 플레이어가 알고 있는 마지막 값 — null = 아직 열리지 않음, "" = 끔(열린 직후 상태), "t:<id>" = 그 트랙
  const sent = useRef<string | null>(null)
  const key = trackId === undefined ? undefined : trackId === null ? "" : `t:${trackId}`
  useEffect(() => {
    if (!ready) {
      sent.current = null
      return
    }
    if (sent.current === null) sent.current = ""
    if (key === undefined || key === sent.current) return
    const prev = sent.current
    sent.current = key
    void enqueue(() => api.playerSetSubtitle(assetId, trackId ?? null)).catch(() => {
      // 플레이어는 이 선택을 받지 못했다 — 알고 있는 값을 되돌려, 같은 선택이 다시 오면 다시 보낸다 (그 사이 다른 값이 보내졌으면 건드리지 않는다)
      if (sent.current === key) sent.current = prev
      toast.error(t("viewer.subtitle.loadFailed"))
    })
  }, [api, assetId, ready, key, trackId, t])
}
