import { Download } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { aiErrorText } from "@/components/ai/labels"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import { useLocalModels } from "@/hooks/useAi"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatBytes } from "@/lib/format"
import type { LocalModel } from "@/lib/ipc-contract-ai"
import { useQueryClient } from "@tanstack/react-query"
import { qk } from "@/hooks/keys"

function ModelRow({ model, onDownload, disabled }: { model: LocalModel; onDownload: () => void; disabled: boolean }) {
  const { t } = useTranslation()
  const percent = Math.round((model.progress ?? 0) * 100)
  return (
    <li className="flex flex-col gap-1.5 px-3 py-2">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{model.name}</span>
        <span className="text-xs text-muted-foreground">
          {t(`ai.model.purpose.${model.purpose}`, { defaultValue: model.purpose })} · {formatBytes(model.sizeBytes)} · {model.license}
        </span>
        <div className="ml-auto flex items-center gap-2">
          <Badge variant={model.state === "failed" ? "destructive" : "outline"} className="rounded-sm">
            {t(`ai.model.state.${model.state}`)}
          </Badge>
          {(model.state === "missing" || model.state === "failed") && (
            <Button size="xs" variant="outline" disabled={disabled} aria-label={t("ai.model.downloadNamed", { name: model.name })} onClick={onDownload}>
              <Download aria-hidden="true" />
              {t("ai.model.download")}
            </Button>
          )}
        </div>
      </div>
      {model.state === "downloading" && (
        <div className="flex items-center gap-2">
          <Progress value={percent} aria-label={t("ai.model.progress", { name: model.name })} />
          <span className="text-xs text-muted-foreground tabular-nums">{percent}%</span>
        </div>
      )}
      {model.state === "failed" && (
        <p role="alert" className="text-xs break-keep text-destructive">
          {model.errorCode === "offline" ? t("ai.model.offline") : aiErrorText(t, model.errorCode ?? "unknown")}
        </p>
      )}
    </li>
  )
}

/** D8: 로컬 모델 목록 — 용도·크기·라이선스·상태, 다운로드 진행률, 오프라인 실패 안내. */
export function LocalModelsList() {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const models = useLocalModels()
  const [error, setError] = useState<string | null>(null)

  const download = async (modelId: string) => {
    setError(null)
    try {
      await api.downloadModel(modelId)
      await qc.invalidateQueries({ queryKey: qk.aiModels })
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  return (
    <div className="flex flex-col gap-1.5">
      <h3 className="text-sm font-medium">{t("ai.model.title")}</h3>
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <ul aria-label={t("ai.model.title")} className="flex flex-col divide-y divide-border rounded-lg border border-border">
        {(models.data ?? []).map((m) => (
          <ModelRow key={m.modelId} model={m} disabled={m.state === "downloading"} onDownload={() => void download(m.modelId)} />
        ))}
      </ul>
    </div>
  )
}
