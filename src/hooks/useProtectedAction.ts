import { useCallback } from "react"
import { useApi } from "@/lib/api"
import { isCommandError } from "@/lib/errors"
import { useProtectStore } from "@/stores"

/**
 * D10 보호 흐름. 인증 면제 시간 안이면 바로 실행하고, 아니면 비밀번호 Dialog 를 거친다.
 * command 가 `auth_required` 로 거절하면(시간 경계·다른 창에서 만료) 비밀번호를 물은 뒤 한 번만 재시도한다.
 * 비밀번호가 설정되지 않았으면 설정 안내 Dialog 를 띄우고 실행하지 않는다.
 *
 * 반환값: 실행했으면 `{ ran: true, value }`, 취소·미설정이면 `{ ran: false }`. command 오류는 그대로 throw 한다.
 */
export type ProtectedResult<T> = { ran: true; value: T } | { ran: false }

export function useProtectedAction() {
  const api = useApi()
  const setPrompt = useProtectStore((s) => s.setPasswordPrompt)
  const setGuide = useProtectStore((s) => s.setPasswordGuideOpen)

  const prompt = useCallback(
    () => new Promise<boolean>((resolve) => setPrompt({ resolve })),
    [setPrompt]
  )

  return useCallback(
    async <T>(command: () => Promise<T>): Promise<ProtectedResult<T>> => {
      const status = await api.getProtectionStatus()
      if (!status.passwordSet) {
        setGuide(true)
        return { ran: false }
      }
      const valid = status.authValidUntil !== null && status.authValidUntil > Date.now()
      if (!valid && !(await prompt())) return { ran: false }
      try {
        return { ran: true, value: await command() }
      } catch (e) {
        if (!(isCommandError(e) && e.code === "auth_required") || !valid) throw e
        // 면제 시간이 끝나 있었다 — 한 번만 비밀번호를 묻고 재시도
        if (!(await prompt())) return { ran: false }
        return { ran: true, value: await command() }
      }
    },
    [api, prompt, setGuide]
  )
}
