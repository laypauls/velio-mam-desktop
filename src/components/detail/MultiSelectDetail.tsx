import { useTranslation } from "react-i18next"
import { BulkMetadataPanel } from "@/components/metadata/BulkMetadataPanel"
import { useMetadataFields } from "@/hooks/useMetadataFields"
import { DetailBody } from "./DetailBody"

/**
 * 콘텐츠를 2개 이상 선택했을 때의 상세 패널 본문 — 사용자 메타데이터 일괄 지정.
 * 정의된 필드가 없으면 일괄 지정할 것이 없으므로 기존처럼 활성 자산의 상세를 보여준다.
 */
export function MultiSelectDetail({ ids, activeId }: { ids: ReadonlySet<number>; activeId: number | null }) {
  const { t } = useTranslation()
  const fields = useMetadataFields()
  if (fields.data && fields.data.length > 0) return <BulkMetadataPanel ids={ids} fields={fields.data} />
  if (activeId === null) return <p className="text-sm break-keep text-muted-foreground">{t("detail.none")}</p>
  return <DetailBody assetId={activeId} />
}
