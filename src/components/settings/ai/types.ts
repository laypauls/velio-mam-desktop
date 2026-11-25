import type { AiSettings, AiSettingsPatch } from "@/lib/ipc-contract-ai"

/** AI 탭 섹션이 공유하는 설정 값과 저장 함수. */
export interface AiCtl {
  settings: AiSettings
  /** 스위치·선택용 — 실패하면 탭 상단에 인라인 오류를 띄운다 */
  patch: (p: AiSettingsPatch) => void
  /** 숫자 입력용 — 실패하면 reject 해 입력 필드가 인라인 오류를 보여준다 */
  commit: (p: AiSettingsPatch) => Promise<unknown>
}
