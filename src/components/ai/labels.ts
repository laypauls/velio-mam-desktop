import type { TFunction } from "i18next"

/** AI 오류 코드(auth_failed, offline, unsupported 등)의 사용자 문구. 알 수 없는 코드는 `ai.error.unknown`. */
export function aiErrorText(t: TFunction, code: string): string {
  return t(`ai.error.${code}`, { defaultValue: t("ai.error.unknown") })
}

/** 언어 코드(ko, en …)의 표시 이름. 목록에 없으면 코드를 그대로 쓴다. */
export function languageText(t: TFunction, code: string): string {
  return t(`ai.lang.${code}`, { defaultValue: code })
}
