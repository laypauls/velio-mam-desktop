import type { SubtitleTrack } from "@/lib/ipc-contract"

export interface PickInput {
  /** 설정 "자막 표시" (꺼져 있으면 자막 없이 시작) */
  visible: boolean
  /** 이번 실행에서 플레이어가 고른 언어 — undefined = 고른 적 없음 */
  sessionLanguage: string | null | undefined
  /** 설정 "자막 기본 언어" — null = 자막 끄고 시작 */
  defaultLanguage: string | null
}

/** 언어를 알 수 없는 트랙이 정확히 1개일 때만 그 트랙 (여러 개면 어느 것인지 알 수 없다) */
function onlyUnknown(tracks: readonly SubtitleTrack[]): SubtitleTrack | null {
  const unknown = tracks.filter((t) => t.language === null)
  return unknown.length === 1 ? unknown[0] : null
}

/**
 * 영상을 열 때 자동으로 고를 자막 트랙 id (D4·D7·D10). 규칙 순서:
 * 표시 꺼짐 → 없음 / 세션 언어(같은 언어 첫 트랙, 알 수 없음이면 알 수 없는 트랙 1개) / 기본 언어 없음(끄고 시작) → 없음 /
 * 기본 언어와 같은 첫 트랙 / 알 수 없는 트랙 1개 / 첫 트랙 (`tracks` 는 Rust 정렬 순).
 */
export function pickTrack(tracks: readonly SubtitleTrack[], { visible, sessionLanguage, defaultLanguage }: PickInput): string | null {
  if (!visible || tracks.length === 0) return null
  if (sessionLanguage !== undefined) {
    const hit = sessionLanguage === null ? onlyUnknown(tracks) : tracks.find((t) => t.language === sessionLanguage)
    if (hit) return hit.id
  }
  if (defaultLanguage === null) return null
  const byDefault = tracks.find((t) => t.language === defaultLanguage)
  if (byDefault) return byDefault.id
  return (onlyUnknown(tracks) ?? tracks[0]).id
}
