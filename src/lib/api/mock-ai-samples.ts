import type { Transcript, TranscriptSegment } from "@/lib/ipc-contract-ai"

export interface AiSample {
  description: string
  keywords: string[]
  ocrText: string
  summary: string
}

/** 합성 분석 결과 — 자산 id 로 결정적으로 고른다. */
const SAMPLES: AiSample[] = [
  { description: "해변에서 노을을 바라보는 사람들", keywords: ["해변", "노을", "여름"], ocrText: "부산 해운대", summary: "저녁 무렵 해변의 풍경을 담은 장면입니다." },
  { description: "회의실에서 발표 자료를 보며 토론하는 팀", keywords: ["회의", "발표", "사무실"], ocrText: "2026 분기 보고", summary: "팀이 분기 실적을 논의하는 회의 장면입니다." },
  { description: "시장 골목에서 음식을 파는 상인과 손님", keywords: ["시장", "음식", "거리"], ocrText: "국밥 8,000원", summary: "활기찬 전통시장의 모습입니다." },
  { description: "산 정상에서 내려다본 구름 낀 풍경", keywords: ["산", "구름", "풍경"], ocrText: "", summary: "구름 사이로 능선이 이어지는 풍경입니다." },
]

export function sampleFor(assetId: number): AiSample {
  return SAMPLES[assetId % SAMPLES.length]
}

const LINES = ["안녕하세요, 오늘 회의를 시작하겠습니다.", "먼저 지난주 진행 상황을 공유드립니다.", "일정은 예정대로 진행되고 있습니다.", "다음 단계는 검토 후 결정하겠습니다.", "질문 있으시면 말씀해 주세요.", "이상으로 마치겠습니다. 감사합니다."]

/** 길이를 7초 구간으로 나눈 STT 결과 (최대 6구간). 원문 언어는 ko. */
export function transcriptFor(assetId: number, durationMs: number | null, createdAt: number): Transcript {
  const total = Math.max(7000, durationMs ?? 21_000)
  const count = Math.min(LINES.length, Math.max(2, Math.floor(total / 7000)))
  const segments: TranscriptSegment[] = []
  for (let i = 0; i < count; i++) {
    segments.push({ startMs: i * 7000, endMs: (i + 1) * 7000, text: LINES[(i + assetId) % LINES.length] })
  }
  return { language: "ko", text: segments.map((s) => s.text).join(" "), segments, engine: "whisper-base", createdAt }
}
