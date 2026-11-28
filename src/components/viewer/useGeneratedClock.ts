import { useEffect, useState } from "react"
import type { SubtitleSegment } from "@/lib/ipc-contract-ai"
import { findActiveSegment } from "./generatedSegments"

/** libmpv 위치 이벤트는 호스트가 5Hz(200ms)로 제한한다 — 마지막 이벤트 이후 이 시간까지만 앞으로 보간한다 (이벤트가 끊겨도 자막이 앞서 가지 않게) */
export const MAX_EXTRAPOLATE_MS = 600

/**
 * `<video>` 의 현재 시각에 맞는 생성된 자막 구간. `timeupdate`(약 250ms)는 거칠어서 재생 중에만 rAF 로 `currentTime` 을 읽고,
 * 탐색·일시정지·메타데이터 로드 때도 한 번 갱신한다. 같은 구간이면 상태를 바꾸지 않아 다시 그리지 않는다.
 */
export function useVideoSegment(el: HTMLMediaElement | null, segments: readonly SubtitleSegment[]): SubtitleSegment | null {
  const [active, setActive] = useState<SubtitleSegment | null>(null)
  useEffect(() => {
    if (!el || segments.length === 0) {
      setActive(null)
      return
    }
    let raf: number | null = null
    const sync = () => setActive(findActiveSegment(segments, el.currentTime * 1000))
    const loop = () => {
      sync()
      raf = !el.paused && !el.ended ? requestAnimationFrame(loop) : null
    }
    const start = () => {
      if (raf === null) loop()
    }
    sync()
    if (!el.paused) start()
    el.addEventListener("play", start)
    el.addEventListener("playing", start)
    for (const e of ["seeked", "pause", "ended", "loadedmetadata"]) el.addEventListener(e, sync)
    return () => {
      if (raf !== null) cancelAnimationFrame(raf)
      el.removeEventListener("play", start)
      el.removeEventListener("playing", start)
      for (const e of ["seeked", "pause", "ended", "loadedmetadata"]) el.removeEventListener(e, sync)
    }
  }, [el, segments])
  return active
}

/**
 * libmpv 경로의 생성된 자막 구간 — 위치(`timeMs`)는 `player://state` 이벤트(최대 5Hz)로만 갱신되므로
 * 재생 중에는 마지막 위치 + 경과 시간 × 배속으로 rAF 마다 보간한다. 정지·끝이면 이벤트 위치 그대로.
 */
export function useMpvSegment(segments: readonly SubtitleSegment[], timeMs: number, playing: boolean, rate: number): SubtitleSegment | null {
  const [active, setActive] = useState<SubtitleSegment | null>(null)
  useEffect(() => {
    if (segments.length === 0) {
      setActive(null)
      return
    }
    const base = performance.now()
    const at = () => timeMs + Math.min(performance.now() - base, MAX_EXTRAPOLATE_MS) * rate
    setActive(findActiveSegment(segments, timeMs))
    if (!playing) return
    let raf = 0
    const loop = () => {
      setActive(findActiveSegment(segments, at()))
      raf = requestAnimationFrame(loop)
    }
    raf = requestAnimationFrame(loop)
    return () => cancelAnimationFrame(raf)
  }, [segments, timeMs, playing, rate])
  return active
}
