import type { RangeKeyHandlers } from "./playRangeKeys"
import type { SceneKeyHandlers } from "./sceneKeys"
import { SEEK_STEP_SEC } from "./viewerMath"

/** 음량 키(↑/↓) 한 번의 변화량 — 슬라이더 step 과 같다 */
export const VOLUME_STEP = 0.05

export interface PlayerKeyActions {
  /** 조작할 대상(요소/세션)이 준비되지 않았으면 false — 키를 처리하지 않는다 */
  ready: boolean
  isVideo: boolean
  togglePlay: () => void
  /** 현재 위치 기준 상대 이동(초) */
  seekBy: (deltaSec: number) => void
  step: (dir: 1 | -1) => void
  toggleMute: () => void
  /** 현재 표시 음량(음소거면 0) 기준 상대 변경 — 결과는 0..1 로 맞춘다 */
  volumeBy: (delta: number) => void
  /** Windows 시스템 음량 상대 변경(Shift+↑/↓) — 앱 음량으로 대체하지 않는다 */
  systemVolumeBy: (delta: number) => void
  /** 전체화면 토글(F) — 없으면 F 를 처리하지 않는다 */
  toggleFullscreen?: () => void
  /** 재생 구간 키(프리셋별) — 없으면 구간 키를 처리하지 않는다 */
  ranges?: RangeKeyHandlers
  /** 장면전환점 키(`Ctrl+←/→` 이동, `C` 추가) — 영상에서만 넘긴다. 없으면 처리하지 않는다 */
  scenes?: SceneKeyHandlers
  /** 단독 보기의 이전/다음 콘텐츠(Shift+P/N) — 연속 재생 중에는 `usePlaybackKeys` 가 처리하므로 넘기지 않는다. 없으면 처리하지 않는다 */
  contentNav?: { canPrev: boolean; canNext: boolean; onPrev: () => void; onNext: () => void }
}

export type PlayerKeyAction =
  | { type: "togglePlay" }
  | { type: "seekBy"; delta: number }
  | { type: "step"; dir: 1 | -1 }
  | { type: "toggleMute" }
  | { type: "volumeBy"; delta: number }
  | { type: "systemVolumeBy"; delta: number }
  | { type: "toggleFullscreen" }

/** `code` 는 물리 키 — 한글 IME 에서 `key` 가 "ㅡ" 가 되는 문자 키는 code 로 판별한다 */
type KeyLike = { key: string; code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }

/** 키 입력 → 플레이어 동작. Ctrl/Alt/Meta 와 함께면 null, Shift 와는 Shift+↑/↓(시스템 음량)만 — 그 밖의 Shift 조합(Shift+N/P·구간 키)은 다른 처리기 몫. `,` `.` 는 영상에서만 */
export function playerKeyAction(e: KeyLike, isVideo: boolean): PlayerKeyAction | null {
  if (e.ctrlKey || e.altKey || e.metaKey) return null
  if (e.shiftKey) {
    if (e.key === "ArrowUp") return { type: "systemVolumeBy", delta: VOLUME_STEP }
    return e.key === "ArrowDown" ? { type: "systemVolumeBy", delta: -VOLUME_STEP } : null
  }
  switch (e.key) {
    case " ":
      return { type: "togglePlay" }
    case "ArrowLeft":
      return { type: "seekBy", delta: -SEEK_STEP_SEC }
    case "ArrowRight":
      return { type: "seekBy", delta: SEEK_STEP_SEC }
    case "ArrowUp":
      return { type: "volumeBy", delta: VOLUME_STEP }
    case "ArrowDown":
      return { type: "volumeBy", delta: -VOLUME_STEP }
    case ",":
      return isVideo ? { type: "step", dir: -1 } : null
    case ".":
      return isVideo ? { type: "step", dir: 1 } : null
    default:
      if (e.code === "KeyF") return { type: "toggleFullscreen" }
      return e.code === "KeyM" ? { type: "toggleMute" } : null
  }
}

export function runPlayerAction(action: PlayerKeyAction, a: PlayerKeyActions): void {
  switch (action.type) {
    case "togglePlay":
      return a.togglePlay()
    case "seekBy":
      return a.seekBy(action.delta)
    case "step":
      return a.step(action.dir)
    case "toggleMute":
      return a.toggleMute()
    case "volumeBy":
      return a.volumeBy(action.delta)
    case "systemVolumeBy":
      return a.systemVolumeBy(action.delta)
    case "toggleFullscreen":
      return a.toggleFullscreen?.()
  }
}

/** 부동소수 누적 오차 없이 0..1 로 맞춘 새 음량 (0.05 단위) */
export function nextVolume(current: number, delta: number): number {
  return Math.min(1, Math.max(0, Math.round((current + delta) * 100) / 100))
}

/** 연속 재생 이동 — 다음 / 이전 */
export type PlaybackKeyAction = "next" | "prev"

/** Shift+N / Shift+P (물리 키 기준 — 한글 IME 에서도 동작, Ctrl/Alt/Meta 와 함께면 null). 플레이어 단축키는 Shift 와 함께면 null 이라 겹치지 않는다. */
export function playbackKeyAction(e: KeyLike): PlaybackKeyAction | null {
  if (!e.shiftKey || e.ctrlKey || e.altKey || e.metaKey) return null
  return e.code === "KeyN" ? "next" : e.code === "KeyP" ? "prev" : null
}
