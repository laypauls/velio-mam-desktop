import type { CapturedAtFallback } from "@/lib/ipc-contract"

/** 촬영일이 없을 때 대체 설정 기본값 — 백엔드 기본과 같다 */
export const DEFAULT_CAPTURED_AT_FALLBACK: CapturedAtFallback = "modified"

/**
 * 화면에 보일 촬영일 — 설정 `capturedAtFallback` 이 "modified" 이고 촬영일이 없으면 수정일로 대체(`fallback: true`).
 * "none" 이면 촬영일 없음(`at: null`) 그대로.
 */
export function effectiveCapturedAt(
  a: { capturedAt: number | null; modifiedAt: number },
  mode: CapturedAtFallback,
): { at: number | null; fallback: boolean } {
  if (a.capturedAt !== null) return { at: a.capturedAt, fallback: false }
  return mode === "modified" ? { at: a.modifiedAt, fallback: true } : { at: null, fallback: false }
}
