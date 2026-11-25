import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useAssetDetail } from "@/hooks/useAssetDetail"
import { useViewStore } from "@/stores"
import type { DetailTab } from "@/stores/view"
import { AssetInfo } from "./AssetInfo"
import { AiTab } from "./ai/AiTab"
import { SimilarTab } from "./ai/SimilarTab"

/** 상세 패널 본문 — 정보 / AI / 유사 탭. 비활성 탭은 마운트하지 않아 AI 조회는 탭을 열 때만 일어난다. */
export function DetailBody({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const { data, isPending, isError, refetch } = useAssetDetail(assetId)
  const tab = useViewStore((s) => s.detailTab)
  const setTab = useViewStore((s) => s.setDetailTab)

  if (isPending) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="h-64 w-full" />
  }
  if (isError || !data) {
    return (
      <div role="alert" className="flex flex-col items-start gap-2 text-sm text-destructive">
        <span className="break-keep">{t("detail.error")}</span>
        <Button size="sm" variant="outline" onClick={() => void refetch()}>
          {t("common.retry")}
        </Button>
      </div>
    )
  }

  return (
    <Tabs value={tab} onValueChange={(v) => setTab(v as DetailTab)}>
      <TabsList className="w-full">
        <TabsTrigger value="info">{t("detail.tab.info")}</TabsTrigger>
        <TabsTrigger value="ai">{t("detail.tab.ai")}</TabsTrigger>
        <TabsTrigger value="similar">{t("detail.tab.similar")}</TabsTrigger>
      </TabsList>
      <TabsContent value="info" className="pt-2">
        <AssetInfo data={data} />
      </TabsContent>
      <TabsContent value="ai" className="pt-2">
        <AiTab assetId={data.assetId} mediaType={data.mediaType} />
      </TabsContent>
      <TabsContent value="similar" className="pt-2">
        <SimilarTab assetId={data.assetId} mediaType={data.mediaType} />
      </TabsContent>
    </Tabs>
  )
}
