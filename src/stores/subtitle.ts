import { create } from "zustand"

interface SubtitleState {
  /**
   * 이번 실행에서 플레이어가 고른 자막 언어 (D7·D10). `undefined` = 아직 고른 적 없음, `null` = 언어를 알 수 없는 트랙을 골랐다.
   * 다음 영상(연속 재생 포함)에서 기본 언어보다 먼저 적용되며, 앱을 닫으면 사라진다.
   */
  sessionLanguage: string | null | undefined
  setSessionLanguage: (language: string | null) => void
}

export const useSubtitleStore = create<SubtitleState>((set) => ({
  sessionLanguage: undefined,
  setSessionLanguage: (sessionLanguage) => set({ sessionLanguage }),
}))
