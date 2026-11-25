import { useQueryClient } from "@tanstack/react-query"
import { ImageOff, Loader2, ScanSearch } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { aiErrorText } from "@/components/ai/labels"
import { Button } from "@/components/ui/button"
import { qk } from "@/hooks/keys"
import { useUpdateAiSettings } from "@/hooks/useAi"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { useProtectStore } from "@/stores"

/** ② 모델은 있으나 의미 검색이 꺼져 있다 — 켜면 기존 미디어의 임베딩이 백그라운드로 만들어진다. */
export function SemanticOffNotice({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const qc = useQueryClient()
  const openSettings = useProtectStore((s) => s.openSettings)
  const update = useUpdateAiSettings()
  const [error, setError] = useState<string | null>(null)

  const enable = async () => {
    setError(null)
    try {
      await update.mutateAsync({ semanticSearch: { enabled: true } })
      await qc.invalidateQueries({ queryKey: qk.similar(assetId) })
      await qc.invalidateQueries({ queryKey: qk.assetAi(assetId) })
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  return (
    <div role="status" className="flex flex-col items-center gap-1.5 p-4 text-center">
      <ScanSearch aria-hidden="true" className="size-6 text-muted-foreground" />
      <p className="text-sm font-medium break-keep">{t("ai.similar.semanticOffTitle")}</p>
      <p className="text-xs break-keep text-muted-foreground">{t("ai.similar.semanticOffDescription")}</p>
      {error && (
        <p role="alert" className="text-xs break-keep text-destructive">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <Button size="sm" disabled={update.isPending} onClick={() => void enable()}>
          {t("ai.similar.enableSemantic")}
        </Button>
        <Button size="sm" variant="outline" onClick={() => openSettings("ai-local")}>
          {t("ai.similar.openSettings")}
        </Button>
      </div>
    </div>
  )
}

/** ③ 이 자산의 임베딩이 대기·진행 중 */
export function EmbeddingPendingNotice() {
  const { t } = useTranslation()
  return (
    <div role="status" aria-busy="true" className="flex flex-col items-center gap-1.5 p-4 text-center">
      <Loader2 aria-hidden="true" className="size-6 animate-spin text-muted-foreground" />
      <p className="text-sm font-medium break-keep">{t("ai.similar.embeddingPendingTitle")}</p>
    </div>
  )
}

/** ④ 이 자산의 임베딩이 없거나 실패 — 지금 만들기로 한 건만 등록한다. */
export function NoEmbeddingNotice({ assetId, errorCode }: { assetId: number; errorCode: string | null }) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const create = async () => {
    setBusy(true)
    setError(null)
    try {
      await api.analyzeAssets([assetId], ["embedding"], false)
      await qc.invalidateQueries({ queryKey: qk.assetAi(assetId) })
    } catch (e) {
      setError(t(errorKey(e)))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div role="status" className="flex flex-col items-center gap-1.5 p-4 text-center">
      <ImageOff aria-hidden="true" className="size-6 text-muted-foreground" />
      <p className="text-sm font-medium break-keep">{t("ai.similar.noEmbeddingTitle")}</p>
      <p className="text-xs break-keep text-muted-foreground">{t("ai.similar.noEmbeddingDescription")}</p>
      {errorCode && (
        <p className="text-xs break-keep text-destructive">
          {t("ai.similar.embeddingFailed", { reason: aiErrorText(t, errorCode) })}
        </p>
      )}
      {error && (
        <p role="alert" className="text-xs break-keep text-destructive">
          {error}
        </p>
      )}
      <Button size="sm" disabled={busy} onClick={() => void create()}>
        {t("ai.similar.createNow")}
      </Button>
    </div>
  )
}
