import { ChevronRight } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useMetadataTypeLabel } from "@/components/metadata/metadataTypes"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import type { MetadataField, UserFieldFilter } from "@/lib/ipc-contract-metadata"
import type { Presence } from "@/lib/userFieldFilters"
import { buildCondition, initialDraft, type FieldDraft } from "./userFieldDraft"
import { ChoiceInput, KeywordInput, RadioRow, RangeInput } from "./UserFieldInputs"

interface Props {
  field: MetadataField
  /** 이 필드에 적용 중인 조건 (필드당 최대 1개) */
  condition: UserFieldFilter | undefined
  /** 유효한 입력일 때만 호출된다 — null = 이 필드의 조건 제거 */
  onChange: (cond: UserFieldFilter | null) => void
}

/** 사용자 필드 하나의 필터 입력 — 접이식. 값 유무(전체/있음/없음) + 타입별 입력. 변경 즉시 반영. */
export function UserFieldFilterRow({ field, condition, onChange }: Props) {
  const { t } = useTranslation()
  const typeLabel = useMetadataTypeLabel()
  const [open, setOpen] = useState(condition !== undefined)
  const [draft, setDraft] = useState<FieldDraft>(() => initialDraft(condition))
  const built = buildCondition(field, draft)

  const update = (patch: Partial<FieldDraft>) => {
    const next = { ...draft, ...patch }
    setDraft(next)
    const r = buildCondition(field, next)
    if (r.invalid) return
    // 값이 같으면 올리지 않는다 — 새 필터 객체가 만들어지면 갤러리 캐시가 초기화된다
    if (JSON.stringify(r.cond) !== JSON.stringify(condition ?? null)) onChange(r.cond)
  }

  const inputProps = { field, draft, onChange: update }
  return (
    <Collapsible open={open} onOpenChange={setOpen} className="flex flex-col rounded-lg border border-border">
      <CollapsibleTrigger className="group flex w-full items-start gap-1.5 rounded-lg px-2 py-1.5 text-left text-sm hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none">
        <ChevronRight aria-hidden="true" className="mt-0.5 size-4 shrink-0 transition-transform group-data-[state=open]:rotate-90 motion-reduce:transition-none" />
        <span className="min-w-0 flex-1 break-words">{field.name}</span>
        <span className="shrink-0 text-xs break-keep text-muted-foreground">{typeLabel(field.valueType)}</span>
        {condition !== undefined && (
          <span role="img" aria-label={t("metadata.filter.fieldActive")} className="mt-1.5 size-2 shrink-0 rounded-full bg-primary" />
        )}
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div role="group" aria-label={field.name} className="flex flex-col !items-start gap-2 px-2 pb-2">
          <RadioRow<Presence>
            label={t("metadata.filter.presenceLabel", { name: field.name })}
            value={draft.presence}
            onChange={(presence) => update({ presence })}
            options={[
              { value: "any", label: t("metadata.filter.presenceAny") },
              { value: "has", label: t("metadata.filter.presenceHas") },
              { value: "missing", label: t("metadata.filter.presenceMissing") },
            ]}
          />
          {/* 값 있음/없음을 고르면 값 입력은 쓰이지 않는다 — 입력 내용은 보존한 채 비활성 */}
          <fieldset disabled={draft.presence !== "any"} className="m-0 w-full min-w-0 border-0 p-0 disabled:opacity-50">
            {field.valueType === "keyword" && <KeywordInput {...inputProps} />}
            {(field.valueType === "number" || field.valueType === "date") && <RangeInput {...inputProps} invalid={built.invalid} />}
            {(field.valueType === "boolean" || field.valueType === "enum" || field.valueType === "multiEnum") && <ChoiceInput {...inputProps} />}
          </fieldset>
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}
