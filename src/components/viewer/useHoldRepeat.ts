import { useEffect, useRef, type PointerEvent as ReactPointerEvent } from "react"

/** 처음 누른 뒤 반복이 시작되기까지(ms) */
export const HOLD_DELAY_MS = 400
/** 반복 사이 간격(ms) — 이전 이동이 끝난 뒤부터 센다 */
export const HOLD_INTERVAL_MS = 70

/**
 * 버튼을 누르고 있는 동안 동작을 반복한다 (프레임 이동 길게 누르기).
 * - 주 버튼 pointerdown 에서 즉시 1회, 400ms 뒤부터 반복. 반복은 고정 간격 타이머로 쌓지 않고 **이전 이동이 끝난 뒤에만** 다음을 요청한다
 *   (`step` 이 Promise 를 돌려주면 그것이 끝날 때까지 — libVLC IPC 왕복 — 기다리므로 손을 뗀 뒤에 대기 중인 이동이 계속 실행되지 않는다).
 * - pointerup · pointerleave · pointercancel · lostpointercapture · window blur · 언마운트에서 멈춘다.
 * - pointerdown 이 이미 이동했으므로 이어지는 click 은 무시한다. pointer 없이 오는 click(키보드·프로그램)은 1회 이동한다.
 */
export function useHoldRepeat(step: () => void | Promise<unknown>) {
  const stepRef = useRef(step)
  stepRef.current = step
  const s = useRef({ gen: 0, timer: null as ReturnType<typeof setTimeout> | null, repeating: false, inflight: false, suppressClick: false, held: false })

  const stop = useRef(() => {
    const st = s.current
    st.gen++
    st.repeating = false
    if (st.timer !== null) clearTimeout(st.timer)
    st.timer = null
  }).current

  const fire = useRef((gen: number) => {
    const st = s.current
    st.inflight = true
    let result: void | Promise<unknown>
    try {
      result = stepRef.current()
    } catch {
      result = undefined
    }
    void Promise.resolve(result)
      .catch(() => undefined)
      .then(() => {
        st.inflight = false
        if (gen !== st.gen || !st.repeating) return
        st.timer = setTimeout(() => fire(gen), HOLD_INTERVAL_MS)
      })
  }).current

  useEffect(() => {
    window.addEventListener("blur", stop)
    return () => {
      window.removeEventListener("blur", stop)
      stop()
    }
  }, [stop])

  /** 누른 채로 벗어남·취소 — 같은 누름의 click 은 오지 않는다 (손을 뗀 뒤의 leave/lostcapture 는 click 앞에 올 수 있어 건드리지 않는다) */
  const cancel = () => {
    const st = s.current
    if (st.held) st.suppressClick = false
    st.held = false
    stop()
  }

  return {
    onPointerDown: (e: ReactPointerEvent) => {
      if (e.button !== 0 || e.isPrimary === false) return
      stop()
      const st = s.current
      st.suppressClick = true
      st.held = true
      const gen = st.gen
      fire(gen)
      st.timer = setTimeout(() => {
        if (gen !== st.gen) return
        st.repeating = true
        if (!st.inflight) fire(gen)
      }, HOLD_DELAY_MS)
    },
    onPointerUp: () => {
      s.current.held = false
      stop()
    },
    onPointerLeave: cancel,
    onPointerCancel: cancel,
    onLostPointerCapture: stop,
    onClick: () => {
      const st = s.current
      if (st.suppressClick) {
        st.suppressClick = false
        return
      }
      stepRef.current()
    },
  }
}
