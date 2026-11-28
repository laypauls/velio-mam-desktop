import { TriangleAlert } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useSubtitleActions } from "@/hooks/useSubtitleActions"
import type { AnalysisStatus } from "@/lib/ipc-contract-ai"
import { useProtectStore } from "@/stores"

/** 건너뜀 이유 코드 중 문구가 있는 것 — 그 밖의 코드는 일반 문구(`unknown`)로 보인다 */
const KNOWN_REASONS = ["unsupported", "model_missing", "disabled", "asset_inactive"] as const
/** 설정에서 해결하는 이유 — 설정 열기 동작을 함께 보인다 */
const SETTINGS_REASONS: readonly string[] = ["model_missing", "disabled"]

/** 자막 단계가 `skipped` 일 때 이유와 [다시 시도](force=false) 를 보인다. 다른 상태에서는 렌더하지 않는다. */
export function SubtitleSkipNotice({ assetId, status }: { assetId: number; status: AnalysisStatus | undefined }) {
  const { t } = useTranslation()
  const { generate } = useSubtitleActions()
  const openSettings = useProtectStore((s) => s.openSettings)
  if (status?.state !== "skipped") return null
  const code = status.errorCode ?? ""
  const reason = (KNOWN_REASONS as readonly string[]).includes(code) ? code : "unknown"
  return (
    <div role="alert" className="flex flex-col items-start gap-1.5 rounded-lg border border-border bg-muted/40 p-2 text-sm">
      <p className="flex items-start gap-1.5 break-keep">
        <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        {t(`ai.subtitle.skip.${reason}`)}
      </p>
      <div className="flex flex-wrap gap-2">
        <Button size="xs" variant="outline" onClick={() => void generate([assetId], false)}>
          {t("ai.subtitle.retry")}
        </Button>
        {SETTINGS_REASONS.includes(code) && (
          <Button size="xs" variant="outline" onClick={() => openSettings("ai-local")}>
            {t("ai.subtitle.openSettings")}
          </Button>
        )}
      </div>
    </div>
  )
}
