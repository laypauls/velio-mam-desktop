import { Download, ScanSearch } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useModelReadiness } from "@/hooks/useAi"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { useProtectStore } from "@/stores"
import { useQueryClient } from "@tanstack/react-query"

/** 유사 미디어는 CLIP 모델이 필요하다 — 없으면 숨기지 않고 이유와 내려받기 동작을 보여 준다. */
export function SimilarModelsNotice() {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const openSettings = useProtectStore((s) => s.openSettings)
  const { clipModels } = useModelReadiness()
  const [error, setError] = useState<string | null>(null)
  const downloading = clipModels.some((m) => m.state === "downloading")

  const download = async () => {
    setError(null)
    try {
      for (const m of clipModels) if (m.state === "missing" || m.state === "failed") await api.downloadModel(m.modelId)
      await qc.invalidateQueries({ queryKey: qk.aiModels })
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  return (
    <div role="status" className="flex flex-col items-center gap-1.5 p-4 text-center">
      <ScanSearch aria-hidden="true" className="size-6 text-muted-foreground" />
      <p className="text-sm font-medium break-keep">{t("ai.similar.needModelsTitle")}</p>
      <p className="text-xs break-keep text-muted-foreground">{t("ai.similar.needModelsDescription")}</p>
      {downloading && <p className="text-xs break-keep text-muted-foreground">{t("ai.similar.downloading")}</p>}
      {error && (
        <p role="alert" className="text-xs break-keep text-destructive">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button size="sm" disabled={downloading} onClick={() => void download()}>
          <Download aria-hidden="true" />
          {t("ai.similar.downloadModels")}
        </Button>
        <Button size="sm" variant="outline" onClick={() => openSettings("ai-local")}>
          {t("ai.similar.openSettings")}
        </Button>
      </div>
    </div>
  )
}
