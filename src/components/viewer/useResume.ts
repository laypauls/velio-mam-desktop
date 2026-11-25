import { useEffect, useRef, useState } from "react"
import { useSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import { usePlaybackStore, useViewStore } from "@/stores"

export interface ResumeRestore {
  /** 설정과 기록 조회가 끝나 시작 위치를 알 수 있다 — 이어보기를 쓰지 않는 경우(꺼짐·명시 seek)도 곧바로 true */
  resolved: boolean
  /** 복원할 기록 위치(ms). 없음·꺼짐·명시 seek 요청이면 null */
  recordMs: number | null
  /** 기록 조회가 실패했다 — 기록이 있는지 모르므로 사용자가 재생하기 전에는 저장하지 않는다(처음 위치가 기록을 지우지 않게) */
  lookupFailed: boolean
}

/** 이어보기 사용 여부(설정 `resumeEnabled`) — 저장 쪽은 이 값을 실시간으로 본다. 설정을 못 받았으면 꺼진 것으로 본다 */
export function useResumeEnabled(): boolean {
  return useSettings().data?.resumeEnabled === true
}

/**
 * 열 때 복원할 이어보기 위치 조회 (D1·D6). 컴포넌트는 자산마다 마운트되므로(`key`) 자산당 한 번만 조회한다.
 * 우선순위: 명시 seek 요청(`useViewStore.seekMs` 로 열렸거나 열린 뒤 요청이 옴 — AI 자막·검색 결과) > 이어보기 > 기존 동작.
 * 사용 설정은 처음 안 값으로 고정한다 — 열린 뒤 설정이 바뀌어도 이미 정한 시작 위치는 유지된다.
 * 외부 프로그램으로 여는 경로(`ExternalOpenNotice`)는 이 훅을 쓰지 않는다.
 */
export function useResumeRestore(assetId: number): ResumeRestore {
  const api = useApi()
  const settings = useSettings()
  // 연속 재생의 현재 항목은 남아 있는 `seekMs`(이전 미리보기의 시각 지정)와 무관하게 이어보기를 적용한다 (D6)
  const [seekAtOpen] = useState(() => {
    const q = usePlaybackStore.getState()
    const inQueue = (q.status === "playing" || q.status === "paused") && q.currentId === assetId
    return !inQueue && useViewStore.getState().seekMs !== null
  })
  const seekTick = useViewStore((s) => s.seekTick)
  const [tickAtOpen] = useState(seekTick)
  const explicit = seekAtOpen || seekTick !== tickAtOpen
  const settled = settings.data !== undefined || settings.isError
  const latched = useRef<boolean | null>(null)
  if (latched.current === null && settled) latched.current = settings.data?.resumeEnabled === true
  const wanted = latched.current === true && !explicit
  const [found, setFound] = useState<{ ms: number | null; failed: boolean } | null>(null)

  useEffect(() => {
    if (!wanted) return
    let cancelled = false
    api.resumeGet(assetId).then(
      (r) => {
        if (!cancelled) setFound({ ms: r?.positionMs ?? null, failed: false })
      },
      () => {
        if (!cancelled) setFound({ ms: null, failed: true })
      }
    )
    return () => {
      cancelled = true
    }
  }, [api, assetId, wanted])

  if (!settled) return { resolved: false, recordMs: null, lookupFailed: false }
  if (!wanted) return { resolved: true, recordMs: null, lookupFailed: false }
  if (found === null) return { resolved: false, recordMs: null, lookupFailed: false }
  return { resolved: true, recordMs: found.ms, lookupFailed: found.failed }
}
