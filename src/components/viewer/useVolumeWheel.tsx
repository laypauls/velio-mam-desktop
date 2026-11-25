import { MonitorSpeaker, Volume2, VolumeX } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { cn } from "@/lib/utils"
import { wheelNotches } from "./wheelNotches"

/** 음량 표시가 떠 있는 시간(ms) */
export const VOLUME_INDICATOR_MS = 1000

/**
 * 스테이지 위 마우스 휠 → 노치 수(`wheelNotches`: deltaMode 반영, 위 = 양수, 정밀 터치패드는 소수)와 Shift 상태로 바꿔 `onNotches` 에 넘긴다. 누적·정수화는 호출부의 `useVolumeNotches` 가 한다.
 * Shift+휠은 시스템 음량 — Chromium 은 Shift+휠을 가로 스크롤(`deltaX`)로 바꿔 보내기도 하므로 deltaY 가 0 이면 deltaX 를 쓴다(Shift 없는 가로 휠은 무시).
 * 페이지 스크롤을 막아야 하므로 non-passive 네이티브 리스너를 쓴다.
 * `stage` 가 null 이면(요소 미준비) 아무것도 하지 않는다. 모든 입력 경로가 같은 `onNotches`(= 같은 누적기)를 공유한다.
 */
export function useVolumeWheel(stage: HTMLElement | null, onNotches: (notches: number, shift: boolean) => void) {
  const live = useRef(onNotches)
  live.current = onNotches
  useEffect(() => {
    if (!stage) return
    const onWheel = (e: WheelEvent) => {
      const delta = e.deltaY !== 0 ? e.deltaY : e.shiftKey ? e.deltaX : 0
      if (delta === 0) return
      e.preventDefault()
      live.current(wheelNotches(delta, e.deltaMode), e.shiftKey)
    }
    stage.addEventListener("wheel", onWheel, { passive: false })
    return () => stage.removeEventListener("wheel", onWheel)
  }, [stage])
}

/** 표시기 상태 — `level` 이 null 이면 숨김. `system` = 시스템(Windows) 음량, `error` 가 있으면 level 대신 오류 문구 */
export interface VolumeFlash {
  level: number | null
  system: boolean
  error: string | null
}

const HIDDEN: VolumeFlash = { level: null, system: false, error: null }

/** 일시 음량 표시 상태 — `flash(0..1)` 로 띄우고 VOLUME_INDICATOR_MS 뒤 사라진다. `flashSystem` = 시스템 음량, `flashError` = 시스템 음량 오류 문구 */
export function useVolumeIndicator() {
  const [state, setState] = useState<VolumeFlash>(HIDDEN)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(
    () => () => {
      if (timer.current) clearTimeout(timer.current)
    },
    []
  )
  const show = (next: VolumeFlash) => {
    setState(next)
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(() => setState(HIDDEN), VOLUME_INDICATOR_MS)
  }
  return {
    ...state,
    flash: (volume: number) => show({ level: volume, system: false, error: null }),
    flashSystem: (volume: number) => show({ level: volume, system: true, error: null }),
    flashError: (message: string) => show({ level: null, system: true, error: message }),
  }
}

/** 스테이지 위에 잠깐 뜨는 음량 표시 (부모는 `relative`) */
export function VolumeIndicator({ level, system = false, error = null, className }: { level: number | null; system?: boolean; error?: string | null; className?: string }) {
  const { t } = useTranslation()
  if (level === null && error === null) return null
  const percent = Math.round((level ?? 0) * 100)
  // 앱 음량 = 스피커, 시스템 음량 = 모니터 스피커 — 글자 없이 아이콘으로 구분한다 (전체 문구는 스크린리더용)
  const Icon = error !== null ? VolumeX : system ? MonitorSpeaker : level === 0 ? VolumeX : Volume2
  return (
    <div
      role="status"
      className={cn(
        "pointer-events-none absolute top-3 right-3 flex items-center gap-1.5 rounded-md bg-background/80 px-2.5 py-1 text-sm tabular-nums",
        error !== null ? "text-destructive" : "text-foreground whitespace-nowrap",
        className
      )}
    >
      <Icon aria-hidden="true" className="size-4" />
      {error !== null ? (
        error
      ) : (
        <>
          <span aria-hidden="true">{percent}%</span>
          <span className="sr-only">{t(system ? "viewer.systemVolumeLevel" : "viewer.volumeLevel", { percent })}</span>
        </>
      )}
    </div>
  )
}
