import { useEffect, useRef, useState } from "react"
import type { PlayRange } from "@/lib/ipc-contract"
import { fitResumeToRanges } from "./resumeRanges"
import type { ResumeRestore } from "./useResume"

export interface HtmlResume {
  /** 복원 판정이 끝나고 메타데이터가 로드되어 현재 위치가 실제 재생 위치다 — 저장을 시작해도 된다 */
  armed: boolean
  /** 실제로 이동한 위치(ms) — 복원하지 않았으면 null (이어 재생 알림의 기준) */
  restoredMs: number | null
}

/**
 * `<video>`/`<audio>` 의 이어보기 복원 — 메타데이터가 로드된 뒤 기록 위치로 `currentTime` 을 옮긴다. 자산당 한 번만 적용한다.
 * `ranges`(구간만 재생): 구간을 아직 못 받았으면(`undefined`) 받을 때까지 기다리고, D5 규칙으로 보정한다. 전체 재생(`null`)이면 그대로.
 * 복원할 기록이 없거나(꺼짐·명시 seek 포함) 복원이 끝나면 `armed` — 메타데이터가 로드되기 전의 0 위치가 기록을 지우지 않게 한다.
 */
export function useHtmlResume(el: HTMLMediaElement | null, restore: ResumeRestore, ranges: readonly PlayRange[] | null | undefined): HtmlResume {
  const [state, setState] = useState<HtmlResume>({ armed: false, restoredMs: null })
  const rangesRef = useRef(ranges)
  rangesRef.current = ranges
  const done = useRef(false)
  const waitingRanges = restore.recordMs !== null && ranges === undefined

  useEffect(() => {
    if (!el || !restore.resolved || done.current || waitingRanges) return
    const apply = () => {
      if (done.current) return
      done.current = true
      if (restore.recordMs === null) {
        setState({ armed: true, restoredMs: null })
        return
      }
      const target = Math.round(fitResumeToRanges(restore.recordMs, rangesRef.current))
      el.currentTime = target / 1000
      setState({ armed: true, restoredMs: target })
    }
    if (el.readyState >= 1) {
      apply()
      return
    }
    el.addEventListener("loadedmetadata", apply, { once: true })
    return () => el.removeEventListener("loadedmetadata", apply)
  }, [el, restore.resolved, restore.recordMs, waitingRanges])

  return state
}

/** 탐색이 끝날 때마다(`seeked`) 1 씩 증가하는 값 — 이어보기 저장의 "탐색 완료" 신호 */
export function useSeekedCount(el: HTMLMediaElement | null): number {
  const [count, setCount] = useState(0)
  useEffect(() => {
    if (!el) return
    const onSeeked = () => setCount((n) => n + 1)
    el.addEventListener("seeked", onSeeked)
    return () => el.removeEventListener("seeked", onSeeked)
  }, [el])
  return count
}
