import { useState, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { useAiSettings, useUpdateAiSettings } from "@/hooks/useAi"
import { errorKey } from "@/lib/errors"
import type { AiSettingsPatch } from "@/lib/ipc-contract-ai"
import type { AiCtl } from "./types"

/**
 * AI 설정 페이지(외부 AI / 로컬 AI·검색) 공용 래퍼 — AI 설정을 불러와 `AiCtl` 을 만들어 자식에게 넘긴다.
 * 로드 실패는 인라인 오류, 저장 실패(스위치·선택)는 페이지 상단 인라인 오류로 보여준다.
 */
export function AiCtlGate({ children }: { children: (ctl: AiCtl) => ReactNode }) {
  const { t } = useTranslation()
  const settings = useAiSettings()
  const update = useUpdateAiSettings()
  const [error, setError] = useState<string | null>(null)

  if (settings.isError) {
    return (
      <p role="alert" className="text-sm text-destructive">
        {t("ai.settingsLoadError")}
      </p>
    )
  }
  if (!settings.data) return null

  const ctl: AiCtl = {
    settings: settings.data,
    patch: (p: AiSettingsPatch) => {
      setError(null)
      update.mutate(p, { onError: (e) => setError(t(errorKey(e))) })
    },
    commit: update.mutateAsync,
  }

  return (
    <div className="flex flex-col gap-4">
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      {children(ctl)}
    </div>
  )
}
