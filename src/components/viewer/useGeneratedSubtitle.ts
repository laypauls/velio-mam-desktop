import { useCallback, useMemo } from "react"
import { useAssetAi } from "@/hooks/useAi"
import { useSettings, useUpdateSettings } from "@/hooks/useSettings"
import type { SubtitleSegment } from "@/lib/ipc-contract-ai"
import { sortSegments } from "./generatedSegments"

/** 자막 메뉴와 재생 경로가 공유하는 "생성된 자막"(음성 인식 결과) 상태 */
export interface GeneratedSubtitleUi {
  /** 시작 시각 순 구간 — 없으면 빈 배열 */
  segments: readonly SubtitleSegment[]
  /** 원문 언어 코드 (자막이 없으면 null) */
  language: string | null
  /** 표시할 자막이 있다 */
  available: boolean
  /** 설정 `generatedSubtitleVisible` */
  visible: boolean
  /** 실제로 화면에 그린다 — 열기 성공 + 자막 있음 + 표시 켜짐 */
  shown: boolean
  /** 표시 켜기/끄기 — 설정에 저장한다 */
  toggle: () => void
}

const NO_SEGMENTS: readonly SubtitleSegment[] = []

/**
 * 영상 한 편의 생성된 자막. 상세 패널 AI 탭과 같은 조회 캐시(`qk.assetAi`)를 쓰므로 새 IPC 가 없고,
 * 분석 이벤트로 캐시가 바뀌면 재생 중에도 새 구간으로 바뀐다. `enabled=false`(열기 전·실패·음성)면 조회하지 않는다.
 */
export function useGeneratedSubtitle(assetId: number, enabled: boolean): GeneratedSubtitleUi {
  const ai = useAssetAi(enabled ? assetId : null)
  const settings = useSettings()
  const { mutate: saveSettings } = useUpdateSettings({ silent: true })
  const transcript = enabled ? (ai.data?.transcript ?? null) : null
  const raw = transcript?.segments
  const segments = useMemo(() => (raw ? sortSegments(raw) : NO_SEGMENTS), [raw])
  const visible = settings.data?.generatedSubtitleVisible ?? true
  const available = segments.length > 0
  const toggle = useCallback(() => saveSettings({ generatedSubtitleVisible: !visible }), [saveSettings, visible])
  return { segments, language: available ? (transcript?.language ?? null) : null, available, visible, shown: enabled && available && visible, toggle }
}
