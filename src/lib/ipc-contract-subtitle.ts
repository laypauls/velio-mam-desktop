/**
 * 동영상 외부 자막 계약 — `docs/requests/2026-11/20261113-163831-architect-subtitles-design.md` §3.
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 */

export type SubtitleFormat = "srt" | "smi" | "vtt" | "ass"

/** 설정 "자막 기본 언어" 로 고를 수 있는 언어 (D9) */
export type SubtitleLanguage = "ko" | "en" | "ja" | "zh"

/** `language` 는 ISO 639-1 (모르면 null). 표시 문구는 프론트가 `language` + `format` 으로 만든다 */
export interface SubtitleTrack {
  id: string
  language: string | null
  format: SubtitleFormat
  fileName: string
}

export interface SubtitleCue {
  startMs: number
  endMs: number
  /** 일반 텍스트, 줄바꿈 `\n` */
  text: string
}

export interface SubtitleApi {
  subtitleTracks(assetId: number): Promise<SubtitleTrack[]> // subtitle_tracks { assetId } — 정렬된 트랙(없으면 []). 오프라인 → asset_offline
  subtitleCues(assetId: number, trackId: string): Promise<SubtitleCue[]> // subtitle_cues { assetId, trackId } — 없는 트랙 not_found, 빈/512자 초과 trackId invalid_input
  playerSetSubtitle(assetId: number, trackId: string | null): Promise<void> // player_set_subtitle { assetId, trackId } — null = 끔. 열 때마다 초기화되므로 열기 성공 뒤마다 다시 보낸다. 다른 자산·세션 없음은 무시(Ok)
}
