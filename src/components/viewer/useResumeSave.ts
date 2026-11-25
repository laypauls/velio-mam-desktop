import { useEffect, useRef } from "react"
import { useApi } from "@/lib/api"

/** 재생 중 주기 저장 간격 (D2) */
export const RESUME_SAVE_INTERVAL_MS = 10_000
/** 탐색 뒤 위치가 자리 잡기를 기다리는 시간 — 연속 탐색(슬라이더 드래그·키 반복)은 마지막 것만 저장한다 */
export const RESUME_SEEK_SETTLE_MS = 600

interface Options {
  assetId: number
  /** 이어보기 사용(설정) — false 면 아무것도 저장하지 않는다 */
  enabled: boolean
  /**
   * 저장해도 되는 상태 — 열기·복원이 끝나 `positionMs` 가 실제 재생 위치일 때 true.
   * 한 번 true 가 되면 유지한다(일시정지·닫기 저장이 막히지 않게). 열기 직후의 0 위치가 기존 기록을 지우지 않게 하는 문턱이다.
   */
  armed: boolean
  playing: boolean
  /** 현재 위치(ms, 원본 타임라인) */
  positionMs: number
  /** 원본 전체 길이(ms), 모르면 null */
  durationMs: number | null
  /** 탐색이 끝날 때마다 1 씩 증가하는 값 — 바뀌면 `RESUME_SEEK_SETTLE_MS` 뒤 저장한다 */
  seekTick: number
}

/**
 * 이어보기 위치 저장 (D2): 일시정지·탐색 완료·재생 중 10초 주기·항목 전환/언마운트·창 닫기(`pagehide`/`beforeunload`, 최선 노력).
 * 저장 판정(처음 5초·끝 5%/15초·최소 길이)은 Rust 가 하므로 위치를 그대로 보낸다. 실패는 재생에 영향이 없어 삼킨다.
 * 위치·길이는 ref 로 읽어 주기·언마운트 저장이 항상 최신 값을 쓴다. 자산 전환은 컴포넌트가 `key` 로 다시 마운트되어 언마운트 저장으로 처리된다.
 */
export function useResumeSave({ assetId, enabled, armed, playing, positionMs, durationMs, seekTick }: Options): void {
  const api = useApi()
  const live = useRef({ assetId, enabled, positionMs, durationMs })
  live.current = { assetId, enabled, positionMs, durationMs }
  const armedRef = useRef(false)
  if (armed) armedRef.current = true
  const canSave = enabled && armedRef.current

  const saveRef = useRef(() => undefined as void)
  saveRef.current = () => {
    const s = live.current
    if (!s.enabled || !armedRef.current || !Number.isFinite(s.positionMs)) return
    const duration = s.durationMs !== null && Number.isFinite(s.durationMs) && s.durationMs > 0 ? Math.round(s.durationMs) : null
    void api.resumeSave(s.assetId, Math.round(Math.max(0, s.positionMs)), duration).catch(() => undefined)
  }

  // 재생 중 주기 저장
  useEffect(() => {
    if (!playing || !canSave) return
    const id = setInterval(() => saveRef.current(), RESUME_SAVE_INTERVAL_MS)
    return () => clearInterval(id)
  }, [playing, canSave])

  // 일시정지(재생 → 정지 전이)
  const wasPlaying = useRef(false)
  useEffect(() => {
    if (wasPlaying.current && !playing) saveRef.current()
    wasPlaying.current = playing
  }, [playing])

  // 탐색 완료 — 처음 값(0)은 탐색이 아니다
  const lastSeek = useRef(seekTick)
  useEffect(() => {
    if (seekTick === lastSeek.current) return
    lastSeek.current = seekTick
    const id = setTimeout(() => saveRef.current(), RESUME_SEEK_SETTLE_MS)
    return () => clearTimeout(id)
  }, [seekTick])

  // 언마운트(항목 전환·닫기)와 창 닫기
  useEffect(() => {
    const flush = () => saveRef.current()
    window.addEventListener("pagehide", flush)
    window.addEventListener("beforeunload", flush)
    return () => {
      window.removeEventListener("pagehide", flush)
      window.removeEventListener("beforeunload", flush)
      flush()
    }
  }, [])
}
