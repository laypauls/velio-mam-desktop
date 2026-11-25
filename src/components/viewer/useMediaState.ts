import { useEffect, useState } from "react"

export interface MediaState {
  currentTime: number
  duration: number
  paused: boolean
  rate: number
  volume: number
  muted: boolean
}

const EVENTS = ["timeupdate", "durationchange", "loadedmetadata", "play", "pause", "ended", "ratechange", "volumechange", "seeked"] as const

const INITIAL: MediaState = { currentTime: 0, duration: Number.NaN, paused: true, rate: 1, volume: 1, muted: false }

function snapshot(el: HTMLMediaElement): MediaState {
  return {
    currentTime: el.currentTime,
    duration: el.duration,
    paused: el.paused,
    rate: el.playbackRate,
    volume: el.volume,
    muted: el.muted,
  }
}

/** `<video>`/`<audio>` 의 재생 상태를 React state 로 미러링한다 (요소는 callback ref state 로 전달). */
export function useMediaState(el: HTMLMediaElement | null): MediaState {
  const [state, setState] = useState<MediaState>(INITIAL)
  useEffect(() => {
    if (!el) return
    const update = () => setState(snapshot(el))
    update()
    for (const name of EVENTS) el.addEventListener(name, update)
    return () => {
      for (const name of EVENTS) el.removeEventListener(name, update)
    }
  }, [el])
  return state
}
