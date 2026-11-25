import { useState } from "react"
import { useTranslation } from "react-i18next"
import { ClearValueButton, FieldValueInput } from "@/components/metadata/FieldValueInput"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useAssetMetadata, useSetAssetMetadata } from "@/hooks/useAssetMetadata"
import { useMetadataFields } from "@/hooks/useMetadataFields"
import { errorKey } from "@/lib/errors"
import type { MetadataField, MetadataValue } from "@/lib/ipc-contract-metadata"

/**
 * 상세 패널 "사용자 메타데이터" — 필드가 하나도 없으면 섹션 전체를 숨긴다(필드는 선택 기능이라 모든 자산에 빈 제목을 두지 않는다.
 * 필드 만들기 안내는 설정 > 메타데이터 필드의 빈 상태가 맡는다). 오프라인·파일 없음 자산도 메모처럼 편집할 수 있다(MAM 관리 데이터).
 */
export function AssetMetadataSection({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const fields = useMetadataFields()
  if (!fields.data || fields.data.length === 0) return null
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-xs font-medium text-muted-foreground">{t("metadata.detail.title")}</h3>
      <FieldRows key={assetId} assetId={assetId} fields={fields.data} />
    </section>
  )
}

function FieldRows({ assetId, fields }: { assetId: number; fields: MetadataField[] }) {
  const { t } = useTranslation()
  const values = useAssetMetadata(assetId)
  if (values.isPending) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="h-16 w-full" />
  }
  if (values.isError) {
    return (
      <div role="alert" className="flex flex-col items-start gap-2 text-sm text-destructive">
        <span className="break-keep">{t("metadata.detail.loadError")}</span>
        <Button size="sm" variant="outline" onClick={() => void values.refetch()}>
          {t("common.retry")}
        </Button>
      </div>
    )
  }
  const byField = new Map(values.data.map((v) => [v.fieldId, v.value]))
  return (
    <div className="flex flex-col gap-3">
      {fields.map((f) => (
        <FieldRow key={f.fieldId} assetId={assetId} field={f} value={byField.get(f.fieldId) ?? null} />
      ))}
    </div>
  )
}

function FieldRow({ assetId, field, value }: { assetId: number; field: MetadataField; value: MetadataValue | null }) {
  const { t } = useTranslation()
  const save = useSetAssetMetadata()
  const [error, setError] = useState<string | null>(null)

  const commit = (next: MetadataValue | null) => {
    setError(null)
    const patch = next === null ? ({ op: "clear", fieldId: field.fieldId } as const) : ({ op: "set", fieldId: field.fieldId, value: next } as const)
    save.mutate({ assetId, patches: [patch] }, { onError: (e) => setError(t(errorKey(e))) })
  }

  const needsClear = (field.valueType === "date" || field.valueType === "multiEnum") && value !== null
  return (
    <FieldValueInput
      field={field}
      value={value}
      onCommit={commit}
      emptyLabel={t("metadata.detail.none")}
      serverError={error}
      actions={needsClear ? <ClearValueButton name={field.name} onClick={() => commit(null)} /> : undefined}
    />
  )
}
