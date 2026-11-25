import { useEffect, useRef } from "react"
import type { PlayRange } from "@/lib/ipc-contract"
import { AUDIO_LOOKAHEAD_MS, nextRangeAction, startToleranceMs, videoLookaheadMs } from "./playRanges"

/** 같은 목표로 다시 seek 하기까지 기다리는 시간(ms) */
export const RESEEK_GUARD_MS = 500

interface Options {
  isVideo: boolean
  fps: number | null
  /** 마지막 구간 끝에 도달해 멈췄을 때 — 네이티브 ended 이벤트는 오지 않으므로 연속 재생 보고를 호출부가 한다 */
  onEnd: () => void
}

type VideoFrameElement = HTMLMediaElement & {
  requestVideoFrameCallback?: (cb: (now: number, metadata: { mediaTime: number }) => void) => number
  cancelVideoFrameCallback?: (handle: number) => void
}

/**
 * `<video>`/`<audio>` 의 "구간만 재생" 강제. `ranges` 가 배열이면(비어 있지 않음) 재생 중에만 위치를 감시한다:
 * 영상은 `requestVideoFrameCallback`(없으면 rAF) 의 프레임마다, 음성은 rAF 마다 `nextRangeAction` 을 평가해
 * 구간 사이/앞이면 다음 구간 시작으로 이동, 마지막 구간 끝이면 일시정지하고 끝 위치에 두며 `onEnd` 를 부른다 (D8: 구간 밖 탐색 후 재생 → 다음 구간 시작).
 * `null`/`undefined` 이면 아무것도 하지 않는다(전체 재생, D7). 감시 핸들은 일시정지·구간 변경·언마운트 때 취소한다.
 */
export function useRangePlayback(el: HTMLMediaElement | null, ranges: readonly PlayRange[] | null | undefined, { isVideo, fps, onEnd }: Options) {
  const endRef = useRef(onEnd)
  endRef.current = onEnd
  // 값이 같은 새 배열로 효과가 다시 시작되지 않게 내용으로 식별한다
  const key = ranges && ranges.length > 0 ? JSON.stringify(ranges) : null

  useEffect(() => {
    if (!el || key === null) return
    const list = JSON.parse(key) as PlayRange[]
    const lookahead = isVideo ? videoLookaheadMs(fps) : AUDIO_LOOKAHEAD_MS
    const tolerance = isVideo ? startToleranceMs(fps) : 1 // 음성도 부동소수 오차(1443.9999…)는 흡수한다
    const frameEl = el as VideoFrameElement
    const useFrameCallback = isVideo && typeof frameEl.requestVideoFrameCallback === "function"
    let handle: number | null = null

    const cancel = () => {
      if (handle === null) return
      if (useFrameCallback) frameEl.cancelVideoFrameCallback?.(handle)
      else cancelAnimationFrame(handle)
      handle = null
    }
    const schedule = () => {
      if (el.paused) return
      handle = useFrameCallback
        ? frameEl.requestVideoFrameCallback!((_now, meta) => {
            handle = null
            tick(meta.mediaTime * 1000)
          })
        : requestAnimationFrame(() => {
            handle = null
            tick(el.currentTime * 1000)
          })
    }
    // 같은 목표로 다시 seek 하지 않는다 — seek 가 끝나기 전/표시 프레임이 목표보다 이른 동안 프레임마다 재요청하면 영영 진행하지 못한다
    let lastSeek: { toMs: number; at: number } | null = null
    const seekTo = (toMs: number) => {
      if (el.seeking) return
      const at = performance.now()
      if (lastSeek && lastSeek.toMs === toMs && at - lastSeek.at < RESEEK_GUARD_MS) return
      lastSeek = { toMs, at }
      el.currentTime = toMs / 1000
    }
    /** 위치 평가 — 이동했거나 끝났으면 true (끝나면 더 감시하지 않는다) */
    const evaluate = (tMs: number): boolean => {
      const action = nextRangeAction(tMs, list, lookahead, tolerance)
      if (action.kind === "seek") {
        seekTo(action.toMs)
      } else if (action.kind === "end") {
        el.pause()
        el.currentTime = action.atMs / 1000
        endRef.current()
        return true
      }
      return false
    }
    const tick = (tMs: number) => {
      if (el.paused) return
      if (!evaluate(tMs)) schedule()
    }
    const onPlay = () => {
      cancel()
      lastSeek = null
      // 끝에서 다시 재생하면 첫 구간부터, 구간 밖에서 재생하면 다음 구간 시작부터 (D8)
      const action = nextRangeAction(el.currentTime * 1000, list, 0, tolerance)
      if (action.kind === "seek") seekTo(action.toMs)
      else if (action.kind === "end") seekTo(list[0].startMs)
      schedule()
    }
    const onPause = () => {
      cancel()
      lastSeek = null
    }

    // 처음 열었거나 구간을 막 켠 상태에서 첫 구간 앞에 서 있으면 첫 구간 시작으로 (정지 상태 유지)
    const seekToFirstIfBefore = () => {
      if (el.paused && el.currentTime * 1000 + tolerance < list[0].startMs) seekTo(list[0].startMs)
    }
    seekToFirstIfBefore()
    const onMetadata = seekToFirstIfBefore
    el.addEventListener("play", onPlay)
    el.addEventListener("pause", onPause)
    el.addEventListener("loadedmetadata", onMetadata)
    // 구간이 켜졌을 때 이미 재생 중이면(자동 재생·토글) 즉시 평가한다
    if (!el.paused) onPlay()
    return () => {
      cancel()
      el.removeEventListener("play", onPlay)
      el.removeEventListener("pause", onPause)
      el.removeEventListener("loadedmetadata", onMetadata)
    }
  }, [el, key, isVideo, fps])
}
