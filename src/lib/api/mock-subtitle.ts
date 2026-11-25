import type { MamApi, SubtitleApi } from "@/lib/ipc-contract"
import type { MockCtx, MockOptions } from "./mock-ctx"
import { cmdError } from "./mock-search"

/** 글자(유니코드 코드 포인트) 수 — Rust 의 chars().count() 와 같다 */
const MAX_TRACK_ID = 512

/**
 * 외부 자막 mock — 옵션으로 준 자산별 트랙만 돌려준다 (기본: 없음).
 * 서버와 같은 오류: 오프라인·파일 없음 → asset_offline, 빈/512자 초과 trackId → invalid_input, 없는 트랙 → not_found.
 * `playerSetSubtitle` 은 인자만 검증하고 아무것도 하지 않는다.
 */
export function createMockSubtitle({ rec, wait }: Pick<MockCtx, "rec" | "wait">, subtitles: MockOptions["subtitles"]): Pick<MamApi, keyof SubtitleApi> {
  const entries = (assetId: number) => {
    // Rust 와 같은 매핑: 자산 없음(NotFound) 도 asset_offline
    let avail: string
    try {
      avail = rec(assetId).avail
    } catch {
      throw cmdError("asset_offline", "asset is offline")
    }
    if (avail !== "online") throw cmdError("asset_offline", "asset is offline")
    return subtitles?.[assetId] ?? []
  }
  const validTrackId = (trackId: string) => {
    if (trackId === "" || [...trackId].length > MAX_TRACK_ID) throw cmdError("invalid_input", "trackId")
  }
  return {
    subtitleTracks: async (assetId) => {
      await wait()
      return entries(assetId).map((e) => ({ ...e.track }))
    },
    subtitleCues: async (assetId, trackId) => {
      await wait()
      validTrackId(trackId)
      const found = entries(assetId).find((e) => e.track.id === trackId)
      if (!found) throw cmdError("not_found", "subtitle track not found")
      return found.cues.map((c) => ({ ...c }))
    },
    playerSetSubtitle: async (assetId, trackId) => {
      await wait()
      if (trackId !== null) {
        validTrackId(trackId)
        if (!entries(assetId).some((e) => e.track.id === trackId)) throw cmdError("not_found", "subtitle track not found")
      }
    },
  }
}
