import { useCallback } from "react"
import { isCommandError } from "@/lib/errors"
import { useAiStore } from "@/stores"

export type ConsentResult<T> = { ran: true; value: T } | { ran: false }

/**
 * D1 동의 흐름. command 가 `consent_required` 로 거절하면 동의 Dialog 를 띄우고,
 * 동의하면 한 번만 재시도한다. 거절·닫기면 실행하지 않는다 (`ran: false`). 그 밖의 오류는 그대로 throw.
 */
export function useConsentFlow() {
  const setPrompt = useAiStore((s) => s.setConsentPrompt)

  const askConsent = useCallback(() => new Promise<boolean>((resolve) => setPrompt({ resolve })), [setPrompt])

  const withConsent = useCallback(
    async <T>(command: () => Promise<T>): Promise<ConsentResult<T>> => {
      try {
        return { ran: true, value: await command() }
      } catch (e) {
        if (!(isCommandError(e) && e.code === "consent_required")) throw e
        if (!(await askConsent())) return { ran: false }
        return { ran: true, value: await command() }
      }
    },
    [askConsent],
  )

  return { askConsent, withConsent }
}
