import { Sparkles } from "lucide-react"
import { useTranslation } from "react-i18next"
import type { AiCardState } from "@/lib/ipc-contract"

/** 유형 아이콘 우상단 AI 배지 — 완료: ✦(AI 색), 실패: 빨간 점. 없음이면 아무것도 그리지 않는다(폭 변화 없음: absolute). */
export function AiStateBadge({ state }: { state: AiCardState }) {
  const { t } = useTranslation()
  if (state === "none") return null
  const label = t(state === "done" ? "asset.ai.done" : "asset.ai.failed")
  return (
    <span title={label} className="absolute -top-1 -right-1 flex items-center justify-center rounded-full bg-card">
      {state === "done" ? (
        <Sparkles className="size-2.5 fill-current text-ai" aria-hidden />
      ) : (
        <span className="size-2 rounded-full bg-destructive ring-2 ring-card" aria-hidden />
      )}
      <span className="sr-only">{label}</span>
    </span>
  )
}
