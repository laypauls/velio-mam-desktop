import { ImageOff } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { AssetThumb } from "@/components/gallery/AssetThumb"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useSettings } from "@/hooks/useSettings"
import { displayName } from "@/lib/displayName"
import { formatDuration, formatNumber } from "@/lib/format"
import { useAiSettings, useAssetAi, useModelReadiness, useSimilar } from "@/hooks/useAi"
import type { MediaType } from "@/lib/ipc-contract"
import { useProtectStore, useSelectionStore } from "@/stores"
import { SimilarCriteria, SimilarThresholdEmpty } from "./SimilarCriteria"
import { EmbeddingPendingNotice, NoEmbeddingNotice, SemanticOffNotice } from "./SimilarEmbeddingNotices"
import { SimilarModelsNotice } from "./SimilarModelsNotice"

/**
 * D6: 유사 미디어 — 작은 썸네일 격자와 유사도 %, 클릭하면 그 자산을 선택한다.
 * 빈 결과의 원인(모델 없음 / 의미 검색 꺼짐 / 임베딩 대기·없음)을 구분해 안내한다. 음성은 시각 임베딩 단계가 없어 분기하지 않는다.
 */
export function SimilarTab({ assetId, mediaType }: { assetId: number; mediaType?: MediaType }) {
  const { t } = useTranslation()
  const { loaded, clipReady } = useModelReadiness()
  const settings = useAiSettings()
  const semanticOn = settings.data?.semanticSearch.enabled === true
  const ai = useAssetAi(clipReady && semanticOn ? assetId : null)
  const embedding = ai.data?.status.find((s) => s.kind === "embedding")
  const needsEmbedding = mediaType !== "audio"
  const embeddingDone = !needsEmbedding || embedding?.state === "done"
  const app = useSettings().data
  const titleEditEnabled = app?.titleEditEnabled ?? false
  const max = app?.similarMaxResults ?? 24
  const minSimilarity = app?.similarMinSimilarity ?? 0.75
  // 기준(개수·하한)을 읽기 전에는 조회하지 않는다 — 기본값으로 한 번 받고 다시 받는 낭비 방지
  const q = useSimilar(assetId, max, minSimilarity, clipReady && semanticOn && embeddingDone && app !== undefined)
  const selectOnly = useSelectionStore((s) => s.selectOnly)
  const openSettings = useProtectStore((s) => s.openSettings)
  const [criteriaOpen, setCriteriaOpen] = useState(false)
  const total = q.data?.pages[0]?.total ?? 0
  const items = q.data?.pages.flatMap((p) => p.items) ?? []
  const loading = <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="h-40 w-full" />

  if (!loaded) return loading
  if (!clipReady) return <SimilarModelsNotice />
  if (settings.isPending) return loading
  if (!semanticOn) return <SemanticOffNotice assetId={assetId} />
  if (!embeddingDone) {
    if (ai.isPending) return loading
    if (embedding?.state === "queued" || embedding?.state === "running") return <EmbeddingPendingNotice />
    return (
      <NoEmbeddingNotice assetId={assetId} errorCode={embedding?.state === "failed" ? embedding.errorCode : null} />
    )
  }
  if (q.isPending) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="h-40 w-full" />
  }
  if (q.isError) {
    return (
      <div role="alert" className="flex flex-col items-start gap-2 text-sm text-destructive">
        <span className="break-keep">{t("ai.similar.error")}</span>
        <Button size="sm" variant="outline" onClick={() => void q.refetch()}>
          {t("common.retry")}
        </Button>
      </div>
    )
  }
  const criteria = (
    <SimilarCriteria max={max} minSimilarity={minSimilarity} open={criteriaOpen} onOpenChange={setCriteriaOpen} />
  )
  if (items.length === 0 && minSimilarity > 0) {
    return (
      <div className="flex flex-col gap-2">
        {criteria}
        <SimilarThresholdEmpty minSimilarity={minSimilarity} onChangeCriteria={() => setCriteriaOpen(true)} />
      </div>
    )
  }
  if (items.length === 0) {
    return (
      <div role="status" className="flex flex-col items-center gap-1.5 p-4 text-center">
        <ImageOff aria-hidden="true" className="size-6 text-muted-foreground" />
        <p className="text-sm font-medium break-keep">{t("ai.similar.emptyTitle")}</p>
        <p className="text-xs break-keep text-muted-foreground">{t("ai.similar.emptyDescription")}</p>
        <Button size="sm" variant="outline" onClick={() => openSettings("ai-local")}>
          {t("ai.similar.openSettings")}
        </Button>
      </div>
    )
  }
  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        {criteria}
        <span className="text-xs text-muted-foreground tabular-nums">
          {t("ai.similar.count", {
            total: formatNumber(total),
            shown: formatNumber(items.length),
          })}
        </span>
      </div>
      <ul aria-label={t("ai.similar.list")} className="grid grid-cols-3 gap-1.5">
        {items.map((it) => {
          const name = displayName(it, titleEditEnabled)
          return (
            <li key={it.assetId}>
              <button
                type="button"
                title={name}
                aria-label={t("ai.similar.item", {
                  name,
                  percent: Math.round(it.similarity * 100),
                })}
                onClick={() => selectOnly(it.assetId)}
                className="relative block aspect-square w-full overflow-hidden rounded-lg border border-border focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                <AssetThumb asset={it} size={320} />
                {it.mediaType === "video" && it.durationMs !== null && (
                  <span className="absolute bottom-1 left-1 rounded-sm bg-background/90 px-1 text-xs tabular-nums">
                    {formatDuration(it.durationMs)}
                  </span>
                )}
                <span className="absolute right-1 bottom-1 rounded-sm bg-background/90 px-1 text-xs tabular-nums">
                  {Math.round(it.similarity * 100)}%
                </span>
              </button>
            </li>
          )
        })}
      </ul>
      {q.hasNextPage && (
        <Button size="sm" variant="outline" disabled={q.isFetchingNextPage} onClick={() => void q.fetchNextPage()}>
          {t("ai.similar.more")}
        </Button>
      )}
    </div>
  )
}
