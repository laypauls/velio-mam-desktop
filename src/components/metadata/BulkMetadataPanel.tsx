import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { useBulkSetAssetMetadata } from "@/hooks/useAssetMetadata"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import { MAX_BULK_ASSETS, type MetadataField, type MetadataPatch, type MetadataValue } from "@/lib/ipc-contract-metadata"
import { FieldValueInput } from "./FieldValueInput"

/** 필드별 이번 변경 — 목록에 없는 필드는 "변경 안 함" */
type Change = { kind: "set"; value: MetadataValue } | { kind: "clear" }

/**
 * 여러 콘텐츠 선택 시 값 일괄 지정(결정 18) — 현재 값은 조회·표시하지 않는다. 모든 필드가 "변경 안 함"으로 시작하고
 * 입력한 필드는 덮어쓰기, [지우기] 한 필드는 값 삭제, [적용] 은 바꾼 필드만 보낸다. 대상은 선택한 자산 id 목록(최대 10,000).
 */
export function BulkMetadataPanel({ ids, fields }: { ids: ReadonlySet<number>; fields: MetadataField[] }) {
  const { t } = useTranslation()
  const bulk = useBulkSetAssetMetadata()
  const [changes, setChanges] = useState<Record<number, Change>>({})
  // 입력 칸을 처음 상태로 다시 그리기 위한 번호 (취소·적용 후)
  const [rev, setRev] = useState(0)
  const [error, setError] = useState<string | null>(null)

  // 선택이 바뀌면 입력해 둔 변경을 버린다 — 다른 콘텐츠에 실수로 적용되지 않게
  useEffect(() => {
    setChanges({})
    setRev((r) => r + 1)
    setError(null)
  }, [ids])

  const count = ids.size
  const over = count > MAX_BULK_ASSETS
  const patches: MetadataPatch[] = fields.flatMap((f): MetadataPatch[] => {
    const c = changes[f.fieldId]
    if (!c) return []
    return [c.kind === "clear" ? { op: "clear", fieldId: f.fieldId } : { op: "set", fieldId: f.fieldId, value: c.value }]
  })

  const setChange = (fieldId: number, change: Change | null) => {
    setError(null)
    setChanges((prev) => {
      const { [fieldId]: _drop, ...rest } = prev
      return change === null ? rest : { ...rest, [fieldId]: change }
    })
  }
  const revert = (fieldId: number) => {
    setChange(fieldId, null)
    setRev((r) => r + 1)
  }

  // 네이티브 disabled 가 아니라 aria-disabled — 글을 입력하고 바로 [적용] 을 누르면 mousedown 의 blur 로 변경이 확정되는데,
  // 그때 버튼이 아직 disabled 면 클릭이 사라져 두 번 눌러야 한다
  const blocked = over || patches.length === 0 || bulk.isPending
  const apply = () => {
    if (blocked) return
    setError(null)
    bulk.mutate(
      { assetIds: [...ids], patches },
      {
        onSuccess: (changed) => {
          toast.success(t("metadata.detail.bulk.done", { count: changed }))
          setChanges({})
          setRev((r) => r + 1)
        },
        onError: (e) => setError(t(errorKey(e))),
      }
    )
  }

  return (
    <section className="flex flex-col gap-3">
      <p className="text-sm font-medium">{t("metadata.detail.bulk.count", { count })}</p>
      <h3 className="text-xs font-medium text-muted-foreground">{t("metadata.detail.title")}</h3>
      <p className="text-xs break-keep text-muted-foreground">{t("metadata.detail.bulk.hint")}</p>
      <div className="flex flex-col gap-3">
        {fields.map((f) => {
          const c = changes[f.fieldId]
          return c?.kind === "clear" ? (
            <ClearedRow key={`${f.fieldId}-${rev}`} field={f} onCancel={() => revert(f.fieldId)} />
          ) : (
            <FieldValueInput
              key={`${f.fieldId}-${rev}`}
              field={f}
              value={c?.kind === "set" ? c.value : null}
              emptyLabel={t("metadata.detail.bulk.keep")}
              onCommit={(next) => setChange(f.fieldId, next === null ? null : { kind: "set", value: next })}
              actions={
                <>
                  <Button type="button" size="sm" variant="outline" aria-label={t("metadata.detail.bulk.clearField", { name: f.name })} onClick={() => setChange(f.fieldId, { kind: "clear" })}>
                    {t("metadata.detail.bulk.clear")}
                  </Button>
                  {c?.kind === "set" && (
                    <Button type="button" size="sm" variant="ghost" aria-label={t("metadata.detail.bulk.cancelField", { name: f.name })} onClick={() => revert(f.fieldId)}>
                      {t("metadata.detail.bulk.cancel")}
                    </Button>
                  )}
                </>
              }
            />
          )
        })}
      </div>
      {over && (
        <p role="alert" className="text-xs break-keep text-destructive">
          {t("metadata.detail.bulk.limit", { max: formatNumber(MAX_BULK_ASSETS) })}
        </p>
      )}
      {error !== null && (
        <p role="alert" className="text-xs break-keep text-destructive">
          {error}
        </p>
      )}
      <Button type="button" aria-disabled={blocked} className="aria-disabled:cursor-not-allowed aria-disabled:opacity-50" onClick={apply}>
        {bulk.isPending ? t("metadata.detail.bulk.applying") : t("metadata.detail.bulk.apply")}
      </Button>
    </section>
  )
}

function ClearedRow({ field, onCancel }: { field: MetadataField; onCancel: () => void }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-1">
      <span className="text-sm font-medium break-all">{field.name}</span>
      <div className="flex items-center gap-1">
        <span className="flex-1 rounded-md border border-dashed border-input px-2 py-1.5 text-sm break-keep text-muted-foreground">
          {t("metadata.detail.bulk.cleared")}
        </span>
        <Button type="button" size="sm" variant="ghost" aria-label={t("metadata.detail.bulk.cancelField", { name: field.name })} onClick={onCancel}>
          {t("metadata.detail.bulk.cancel")}
        </Button>
      </div>
    </div>
  )
}
