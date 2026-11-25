import { Maximize2 } from "lucide-react"
import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { AssetThumb } from "@/components/gallery/AssetThumb"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import type { AssetDetail } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { AssetActions, unavailableReason } from "./AssetActions"
import { AssetLinks } from "./AssetLinks"
import { AssetMetadataSection } from "./AssetMetadataSection"
import { AssetRating } from "./AssetRating"
import { CollectionMembership } from "./CollectionMembership"
import { CopyTextButton } from "./CopyTextButton"
import { FavoriteButton } from "./FavoriteButton"
import { LocationLine } from "./LocationLine"
import { MemoEditor } from "./MemoEditor"
import { MetadataList } from "./MetadataList"
import { ReasonTooltip } from "./ReasonTooltip"
import { StatusBanner } from "./StatusBanner"
import { TagEditor } from "./TagEditor"
import { TitleEditor } from "./TitleEditor"

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="flex flex-col gap-1.5">
      <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
      {children}
    </section>
  )
}

/** 상세 패널 "정보" 탭 — 썸네일, 상태 배너(문제일 때만), 제목·위치·작업, 메타데이터, 태그·메모·컬렉션. */
export function AssetInfo({ data }: { data: AssetDetail }) {
  const { t } = useTranslation()
  const openPreview = useViewStore((s) => s.openPreview)
  const reason = unavailableReason(data.availability, t)
  return (
    <div className="flex flex-col gap-4">
      <div className="relative aspect-4/3 overflow-hidden rounded-lg border border-border">
        <AssetThumb asset={data} size={1024} />
        <div className="absolute right-2 bottom-2">
          <ReasonTooltip reason={reason}>
            <Button size="sm" variant="secondary" disabled={reason !== null} onClick={() => openPreview(data.assetId)}>
              <Maximize2 aria-hidden="true" />
              {t("detail.enlarge")}
            </Button>
          </ReasonTooltip>
        </div>
      </div>
      <StatusBanner detail={data} />
      <div className="flex flex-col gap-2">
        <div className="flex items-start gap-1">
          <div className="min-w-0 flex-1">
            <TitleEditor
              assetId={data.assetId}
              title={data.title}
              fileName={data.fileName}
              fileNameAction={<CopyTextButton label={t("detail.copyFileName")} text={data.fileName} doneKey="detail.copiedFileName" />}
            />
          </div>
          <FavoriteButton assetId={data.assetId} favorite={data.favorite} />
        </div>
        <AssetRating assetId={data.assetId} rating={data.rating} />
        <LocationLine data={data} />
        <AssetActions assetId={data.assetId} availability={data.availability} />
      </div>
      <MetadataList detail={data} />
      <Separator />
      <Section title={t("detail.tags")}>
        <TagEditor assetId={data.assetId} tags={data.tags} />
      </Section>
      <Section title={t("detail.memo")}>
        <MemoEditor assetId={data.assetId} memo={data.memo} />
      </Section>
      <AssetMetadataSection assetId={data.assetId} />
      <CollectionMembership assetId={data.assetId} memberOf={data.collections} />
      <AssetLinks detail={data} />
    </div>
  )
}
