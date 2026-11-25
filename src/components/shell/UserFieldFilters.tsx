import { ChevronRight } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Badge } from "@/components/ui/badge"
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible"
import { useMetadataFields } from "@/hooks/useMetadataFields"
import type { UserFieldFilter } from "@/lib/ipc-contract-metadata"
import { userFieldsOrUndefined } from "@/lib/userFieldFilters"
import { useSearchStore } from "@/stores"
import { UserFieldFilterRow } from "./UserFieldFilterRow"

/** 필터 Popover 의 "사용자 메타데이터" 섹션 — 정의된 필드(최대 20개)를 접이식으로 나열. 필드가 없으면 그리지 않는다. */
export function UserFieldFilters() {
  const { t } = useTranslation()
  const fields = useMetadataFields().data
  const conds = useSearchStore((s) => s.filters.userFields)
  const patch = useSearchStore((s) => s.patchFilters)
  const [open, setOpen] = useState((conds?.length ?? 0) > 0)
  if (!fields || fields.length === 0) return null

  const byField = new Map((conds ?? []).map((c) => [c.fieldId, c]))
  const setCondition = (fieldId: number, cond: UserFieldFilter | null) => {
    // 빠르게 연속 입력해도 최신 목록 기준으로 바꾼다
    const cur = useSearchStore.getState().filters.userFields ?? []
    const exists = cur.some((c) => c.fieldId === fieldId)
    const next = cond === null ? cur.filter((c) => c.fieldId !== fieldId) : exists ? cur.map((c) => (c.fieldId === fieldId ? cond : c)) : [...cur, cond]
    patch({ userFields: userFieldsOrUndefined(next) })
  }

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="flex flex-col gap-1.5">
      <CollapsibleTrigger className="group flex w-full items-center gap-1.5 rounded-md text-left text-xs font-medium text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none">
        <ChevronRight aria-hidden="true" className="size-4 shrink-0 transition-transform group-data-[state=open]:rotate-90 motion-reduce:transition-none" />
        <span className="break-keep">{t("metadata.filter.title")}</span>
        {byField.size > 0 && (
          <Badge className="rounded-sm px-1.5" aria-label={t("metadata.filter.active", { count: byField.size })}>
            {byField.size}
          </Badge>
        )}
      </CollapsibleTrigger>
      <CollapsibleContent>
        {/* 섹션 안쪽 스크롤 — 스크롤 영역은 relative 를 함께 둔다(숨김 입력이 바깥 조상 기준으로 배치되는 것 방지) */}
        <ul aria-label={t("metadata.filter.title")} className="relative flex max-h-72 flex-col gap-1.5 overflow-y-auto pr-1">
          {fields.map((f) => (
            <li key={f.fieldId}>
              <UserFieldFilterRow field={f} condition={byField.get(f.fieldId)} onChange={(c) => setCondition(f.fieldId, c)} />
            </li>
          ))}
        </ul>
      </CollapsibleContent>
    </Collapsible>
  )
}
