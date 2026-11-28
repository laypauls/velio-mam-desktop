import type { TFunction } from "i18next"
import type { AnalysisKind } from "@/lib/ipc-contract-ai"

/** AI 오류 코드(auth_failed, offline, unsupported 등)의 사용자 문구. 알 수 없는 코드는 `ai.error.unknown`. */
export function aiErrorText(t: TFunction, code: string): string {
  return t(`ai.error.${code}`, { defaultValue: t("ai.error.unknown") })
}

/** 언어 코드(ko, en …)의 표시 이름. 목록에 없으면 코드를 그대로 쓴다. */
export function languageText(t: TFunction, code: string): string {
  return t(`ai.lang.${code}`, { defaultValue: code })
}

/** 분석 종류의 표시 문구 키 — IPC 값 `stt` 는 화면에서 "자막"(`ai.kind.subtitle`)으로 보인다. 종류 문구는 반드시 이 함수로 키를 만든다 */
export function analysisKindKey(kind: AnalysisKind): string {
  return `ai.kind.${kind === "stt" ? "subtitle" : kind}`
}
