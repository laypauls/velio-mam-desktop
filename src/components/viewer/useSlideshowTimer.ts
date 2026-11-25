import { useEffect, useRef, useState } from "react"

/** 진행 표시 갱신 간격 */
const TICK_MS = 200

/**
 * 이미지 슬라이드 타이머 — `active` 인 동안 `seconds` 가 지나면 `onElapsed`. 일시정지(`active` false)하면 남은 시간을 보존하고
 * 재개하면 이어서 센다. `resetKey` 가 바뀌면(다음 항목) 처음부터. 타이머는 화면이 마운트될 때 시작한다(이미지 디코딩 완료를 기다리지 않음).
 * 반환값은 진행률 0..1 (진행 막대용).
 */
export function useSlideshowTimer(active: boolean, seconds: number, resetKey: number, onElapsed: () => void): number {
  const [progress, setProgress] = useState(0)
  /** 이미 보여 준 시간(ms) — 일시정지로 멈춘 동안 유지 */
  const shown = useRef(0)
  const elapsed = useRef(onElapsed)
  elapsed.current = onElapsed

  // 선언 순서가 중요하다: 항목이 바뀌면 아래 타이머 effect 의 정리(누적)가 먼저 돌고, 이 effect 가 0 으로 되돌린 뒤 타이머가 다시 시작된다
  useEffect(() => {
    shown.current = 0
    setProgress(0)
  }, [resetKey])

  useEffect(() => {
    if (!active) return
    const total = seconds * 1000
    const base = shown.current
    const startedAt = Date.now()
    const done = setTimeout(() => elapsed.current(), Math.max(0, total - base))
    const tick = setInterval(() => setProgress(Math.min(1, (base + Date.now() - startedAt) / total)), TICK_MS)
    return () => {
      clearTimeout(done)
      clearInterval(tick)
      shown.current = Math.min(total, base + Date.now() - startedAt)
    }
  }, [active, seconds, resetKey])

  return progress
}
