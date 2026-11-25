import { useSettings } from "./useSettings"

export const DEFAULT_PLAYING_STEP_SEC = 5
/** 설정 > 플레이어 Select 선택지 */
export const PLAYING_STEP_OPTIONS = [1, 2, 3, 5, 10, 15, 30] as const

/** 저장값이 범위(정수 1~60) 밖이면 기본값 */
export function normalizePlayingStep(v: unknown): number {
  return typeof v === "number" && Number.isInteger(v) && v >= 1 && v <= 60 ? v : DEFAULT_PLAYING_STEP_SEC
}

/** 플레이어 이동 버튼·`,` `.` 키가 재생 중에 이동할 초 (설정 `playerPlayingStepSec`) */
export function usePlayingStepSec(): number {
  return normalizePlayingStep(useSettings().data?.playerPlayingStepSec)
}
