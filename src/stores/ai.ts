import { create } from "zustand"

interface AiState {
  /** 외부 전송 동의 Dialog 가 열려 있으면 그 결과(동의 여부)를 기다리는 resolver */
  consentPrompt: { resolve: (agreed: boolean) => void } | null
  /** 일괄 AI 분석 Dialog 대상 자산 */
  bulkAnalyzeIds: number[] | null
  setConsentPrompt: (p: AiState["consentPrompt"]) => void
  openBulkAnalyze: (ids: number[]) => void
  closeBulkAnalyze: () => void
}

/** AI 흐름(동의 Dialog, 일괄 분석 Dialog)의 열림 상태. API 키는 여기에 두지 않는다. */
export const useAiStore = create<AiState>()((set) => ({
  consentPrompt: null,
  bulkAnalyzeIds: null,
  setConsentPrompt: (consentPrompt) => set({ consentPrompt }),
  openBulkAnalyze: (bulkAnalyzeIds) => set({ bulkAnalyzeIds }),
  closeBulkAnalyze: () => set({ bulkAnalyzeIds: null }),
}))
