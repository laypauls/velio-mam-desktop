import { vi } from "vitest"
import type { UiApi } from "@/lib/api"
import type { AssetAiDetail, SubtitleSegment } from "@/lib/ipc-contract-ai"

/** 생성된 자막(음성 인식 결과)만 있는 자산 AI 상세 — `transcript === null` 이면 자막 없음 */
export function aiDetail(segments: SubtitleSegment[] | null, language = "ko"): AssetAiDetail {
  return {
    analysis: null,
    transcript: segments === null ? null : { language, text: segments.map((s) => s.text).join(" "), segments, engine: "whisper-base", createdAt: 0 },
    status: [],
  }
}

/** `getAssetAi` 가 주어진 구간을 돌려주게 한다 (렌더 전 `setup` 에서 호출) */
export function seedTranscript(api: UiApi, segments: SubtitleSegment[] | null, language = "ko") {
  return vi.spyOn(api, "getAssetAi").mockResolvedValue(aiDetail(segments, language))
}

export const SEGMENTS: SubtitleSegment[] = [
  { startMs: 1000, endMs: 2000, text: "생성 하나" },
  { startMs: 3000, endMs: 4000, text: "생성 둘" },
]
