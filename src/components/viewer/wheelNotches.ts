import { useMemo, useRef } from "react"
import { VOLUME_STEP } from "./playerKeys"

/** 이 시간(ms) 동안 휠이 없으면 모아 둔 소수 노치를 버린다 — 오래전 조금 돌린 것이 나중 입력에 더해지지 않게 */
export const WHEEL_IDLE_MS = 400

/** 1노치(= 음량 VOLUME_STEP)에 해당하는 휠 변화량 — WebView2 의 마우스 휠 한 칸은 pixel 모드 100 */
export const WHEEL_PIXELS_PER_NOTCH = 100
/** line 모드(일부 장치·설정)는 한 칸 = 3줄 */
export const WHEEL_LINES_PER_NOTCH = 3
/** page 모드는 한 페이지 = 1노치 */
export const WHEEL_PAGES_PER_NOTCH = 1

/**
 * DOM 휠 이벤트의 `deltaY` 를 노치 수로 바꾼다 (양수 = 위 = 음량 증가, 정밀 터치패드는 소수).
 * `deltaMode`: 0 = pixel, 1 = line, 2 = page. 알 수 없는 모드는 pixel 로 본다.
 */
export function wheelNotches(deltaY: number, deltaMode: number): number {
  const per = deltaMode === 1 ? WHEEL_LINES_PER_NOTCH : deltaMode === 2 ? WHEEL_PAGES_PER_NOTCH : WHEEL_PIXELS_PER_NOTCH
  return -deltaY / per
}

export type NotchAccumulator = ((notches: number) => number) & { /** 모아 둔 나머지를 버린다 */ reset: () => void }

/**
 * 소수 노치 누적기 — 정수 노치가 찰 때마다 그 정수를 돌려주고(0 이면 아직), 나머지는 다음 입력으로 넘긴다.
 * 방향이 바뀌거나 WHEEL_IDLE_MS 이상 멈췄다 들어온 입력은 모아 둔 나머지를 버리고 새로 시작한다.
 * 유한하지 않은 값·0 은 무시한다.
 */
export function createNotchAccumulator(now: () => number = () => Date.now()): NotchAccumulator {
  let acc = 0
  let lastAt = 0
  const accumulate = (notches: number) => {
    if (!Number.isFinite(notches) || notches === 0) return 0
    const at = now()
    if (at - lastAt > WHEEL_IDLE_MS || Math.sign(notches) !== Math.sign(acc)) acc = 0
    lastAt = at
    acc += notches
    // 0.1 × 10 = 0.9999999999999999 같은 부동소수 오차로 노치가 한 입력 늦어지지 않도록 아주 작은 허용치를 둔다
    const whole = Math.trunc(acc + Math.sign(acc) * 1e-9)
    acc -= whole
    return whole
  }
  return Object.assign(accumulate, {
    reset: () => {
      acc = 0
    },
  })
}

/**
 * 휠 음량의 단일 누적 지점 — 웹 stage 휠(`useVolumeWheel`)과 libVLC 네이티브 입력(`useVlcInput`)이 이 함수 하나를 함께 써서
 * 어느 경로로 들어오든 같은 누적·리셋 규칙을 따른다. 반환 함수는 안정적이며 (소수) 노치와 Shift 상태를 받아
 * 정수 노치마다 Shift 없으면 `volumeBy(n × VOLUME_STEP)`(앱 음량), Shift 면 `systemVolumeBy(n × VOLUME_STEP)`(시스템 음량)를 부른다.
 * 앱·시스템 누적기는 따로 두고, 한쪽을 쓰면 다른 쪽의 나머지를 버려 섞어 돌려도 서로의 소수 노치가 새지 않는다.
 */
export function useVolumeNotches(volumeBy: (delta: number) => void, systemVolumeBy?: (delta: number) => void): (notches: number, shift?: boolean) => void {
  const live = useRef({ volumeBy, systemVolumeBy })
  live.current = { volumeBy, systemVolumeBy }
  return useMemo(() => {
    const app = createNotchAccumulator()
    const system = createNotchAccumulator()
    return (notches: number, shift = false) => {
      const [mine, other] = shift ? [system, app] : [app, system]
      other.reset()
      const whole = mine(notches)
      if (whole === 0) return
      const target = shift ? live.current.systemVolumeBy : live.current.volumeBy
      target?.(whole * VOLUME_STEP)
    }
  }, [])
}
