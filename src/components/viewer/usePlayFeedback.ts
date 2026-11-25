import { useEffect, useRef, useState } from "react"
import { clampTime } from "./viewerMath"

/** 재생/일시정지 아이콘이 떠 있는 시간(ms) — index.css 의 feedback-pop 길이와 같다 */
export const TOGGLE_FEEDBACK_MS = 600
/** 마지막 이동 입력 뒤 앞으로/뒤로 표시가 남는 시간(ms) */
export const SEEK_FEEDBACK_MS = 700

export interface ToggleFeedback {
  /** 입력 뒤의 상태 — true 면 일시정지(❚❚ 표시), false 면 재생(▶ 표시) */
  paused: boolean
  /** 같은 입력이 반복돼도 애니메이션을 다시 시작하도록 key 로 쓴다 */
  tick: number
}

export interface SeekFeedback {
  dir: 1 | -1
  /** 같은 방향으로 이어진 이동의 누적 초(절댓값, 반올림 전) */
  sec: number
  tick: number
}

/** 요청한 이동량 중 실제로 움직인 초 — 0 이하/끝 경계에서 클램프된 만큼 줄어든다 */
export function movedBy(current: number, delta: number, duration: number): number {
  return clampTime(current + delta, duration) - current
}

/**
 * 플레이어 중앙 상태 표시 — 사용자 입력 경로(무대 클릭·키·컨트롤 바 버튼)에서만 호출한다 (상태 전이 이벤트로 부르지 않는다).
 * `flashToggle(nextPaused)` 는 입력 뒤 상태의 아이콘을, `flashSeek(movedSec)` 는 방향별 누적 초를 띄운다 (0 은 무시, 방향이 바뀌면 누적 리셋).
 */
export function usePlayFeedback() {
  const [toggle, setToggle] = useState<ToggleFeedback | null>(null)
  const [seek, setSeek] = useState<SeekFeedback | null>(null)
  const toggleTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const seekTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const acc = useRef({ dir: 0, sec: 0, tick: 0 })
  const toggleTick = useRef(0)
  useEffect(
    () => () => {
      if (toggleTimer.current) clearTimeout(toggleTimer.current)
      if (seekTimer.current) clearTimeout(seekTimer.current)
    },
    []
  )
  const flashToggle = (nextPaused: boolean) => {
    toggleTick.current += 1
    setToggle({ paused: nextPaused, tick: toggleTick.current })
    if (toggleTimer.current) clearTimeout(toggleTimer.current)
    toggleTimer.current = setTimeout(() => setToggle(null), TOGGLE_FEEDBACK_MS)
  }
  const flashSeek = (movedSec: number) => {
    if (!Number.isFinite(movedSec) || movedSec === 0) return
    const dir = movedSec > 0 ? 1 : -1
    const a = acc.current
    a.sec = a.dir === dir ? a.sec + Math.abs(movedSec) : Math.abs(movedSec)
    a.dir = dir
    a.tick += 1
    setSeek({ dir, sec: a.sec, tick: a.tick })
    if (seekTimer.current) clearTimeout(seekTimer.current)
    seekTimer.current = setTimeout(() => {
      acc.current = { dir: 0, sec: 0, tick: acc.current.tick }
      setSeek(null)
    }, SEEK_FEEDBACK_MS)
  }
  return { toggle, seek, flashToggle, flashSeek }
}
