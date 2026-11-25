import type { UpdateError } from "@/lib/ipc-contract"

export const UPDATE_ERROR_CODES: readonly UpdateError["code"][] = [
  "check_failed",
  "not_checked",
  "previous_installer_missing",
  "prepare_failed",
  "install_failed",
]

function knownCode(code: unknown): UpdateError["code"] | null {
  return UPDATE_ERROR_CODES.find((c) => c === code) ?? null
}

/** reject 값에서 `UpdateError.code` 를 읽는다. 알 수 없으면 null. */
export function updateErrorCode(e: unknown): UpdateError["code"] | null {
  if (typeof e !== "object" || e === null || !("code" in e)) return null
  return knownCode((e as { code: unknown }).code)
}

/** 사용자 노출 메시지 번역 키 (코드 문자열 또는 reject 값) — 알 수 없으면 `update.error.unknown`. */
export function updateErrorKey(codeOrError: unknown): string {
  const code = typeof codeOrError === "string" ? knownCode(codeOrError) : updateErrorCode(codeOrError)
  return `update.error.${code ?? "unknown"}`
}
