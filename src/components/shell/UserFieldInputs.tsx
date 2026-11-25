import { X } from "lucide-react"
import { useId, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { DateInput } from "@/components/common/DateInput"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import { MAX_KEYWORD_CHARS, type MetadataField } from "@/lib/ipc-contract-metadata"
import { normalizeKeyword } from "@/lib/userFieldFilters"
import { nameKey } from "@/components/metadata/metadataTypes"
import type { FieldDraft } from "./userFieldDraft"

interface InputProps {
  field: MetadataField
  draft: FieldDraft
  /** 바뀐 입력 상태를 합쳐 올린다 */
  onChange: (patch: Partial<FieldDraft>) => void
}

/** 라벨이 붙은 라디오 한 줄 묶음 (그룹 이름 = aria-label) */
export function RadioRow<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: T
  options: { value: T; label: string }[]
  onChange: (v: T) => void
}) {
  const id = useId()
  return (
    <RadioGroup aria-label={label} value={value} onValueChange={(v) => onChange(v as T)} className="flex flex-wrap gap-x-3 gap-y-1">
      {options.map((o) => (
        <div key={o.value} className="flex items-center gap-1.5">
          <RadioGroupItem id={`${id}-${o.value}`} value={o.value} />
          <Label htmlFor={`${id}-${o.value}`} className="font-normal break-keep">
            {o.label}
          </Label>
        </div>
      ))}
    </RadioGroup>
  )
}

/** keyword — 값을 하나씩 추가하는 칩 입력 (여러 개면 그중 하나라도 일치). 값 목록 제안은 없다(결정 10). */
export function KeywordInput({ field, draft, onChange }: InputProps) {
  const { t } = useTranslation()
  const add = () => {
    const v = normalizeKeyword(draft.keywordText)
    if (v === "") {
      if (draft.keywordText !== "") onChange({ keywordText: "" })
      return
    }
    const dup = draft.keywords.some((k) => nameKey(k) === nameKey(v))
    onChange({ keywordText: "", keywords: dup ? draft.keywords : [...draft.keywords, v] })
  }
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter" || e.nativeEvent.isComposing) return
    e.preventDefault()
    add()
  }
  return (
    <div className="flex flex-col !items-start gap-1.5">
      <div className="flex w-full items-center gap-1.5">
        <Input
          aria-label={t("metadata.filter.keywordInput", { name: field.name })}
          placeholder={t("metadata.filter.keywordPlaceholder")}
          maxLength={MAX_KEYWORD_CHARS}
          value={draft.keywordText}
          className="min-w-0 flex-1"
          onChange={(e) => onChange({ keywordText: e.target.value })}
          onKeyDown={onKeyDown}
          onBlur={add}
        />
        <Button type="button" variant="outline" size="sm" onClick={add}>
          {t("metadata.filter.keywordAdd")}
        </Button>
      </div>
      {draft.keywords.length > 0 && (
        <ul aria-label={t("metadata.filter.keywordList", { name: field.name })} className="flex flex-wrap gap-1">
          {draft.keywords.map((k) => (
            <li key={k} className="flex max-w-full items-center gap-0.5 rounded-sm bg-secondary py-0.5 pr-0.5 pl-1.5 text-xs text-secondary-foreground">
              <span className="min-w-0 break-all">{k}</span>
              <button
                type="button"
                aria-label={t("metadata.filter.keywordRemove", { value: k })}
                onClick={() => onChange({ keywords: draft.keywords.filter((x) => x !== k) })}
                className="shrink-0 rounded-sm p-0.5 hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:outline-none"
              >
                <X aria-hidden="true" className="size-3" />
              </button>
            </li>
          ))}
        </ul>
      )}
      <p className="text-xs break-keep text-muted-foreground">{t("metadata.filter.keywordHint")}</p>
    </div>
  )
}

/** number / date — 최소~최대 (한쪽만 입력해도 됨). 역전된 범위는 인라인 오류 + 적용하지 않음. */
export function RangeInput({ field, draft, invalid, onChange }: InputProps & { invalid: boolean }) {
  const { t } = useTranslation()
  const errId = useId()
  const date = field.valueType === "date"
  const common = {
    "aria-invalid": invalid || undefined,
    "aria-describedby": invalid ? errId : undefined,
    className: "min-w-0 flex-1",
  } as const
  const box = (ariaLabel: string, value: string, set: (v: string) => void) =>
    date ? (
      <DateInput {...common} aria-label={ariaLabel} value={value} onValueChange={set} />
    ) : (
      <Input {...common} type="number" step="any" aria-label={ariaLabel} value={value} onChange={(e) => set(e.target.value)} />
    )
  return (
    <div className="flex flex-col !items-start gap-1">
      <div className="flex w-full items-center gap-2">
        {box(t(date ? "metadata.filter.dateFrom" : "metadata.filter.min", { name: field.name }), draft.min, (min) => onChange({ min }))}
        <span aria-hidden="true">~</span>
        {box(t(date ? "metadata.filter.dateTo" : "metadata.filter.max", { name: field.name }), draft.max, (max) => onChange({ max }))}
      </div>
      {invalid && (
        <p id={errId} role="alert" className="text-xs break-keep text-destructive">
          {t("filter.rangeInvalid")}
        </p>
      )}
    </div>
  )
}

/** boolean 3상태 + enum/multiEnum 체크 목록(+ any/all) */
export function ChoiceInput({ field, draft, onChange }: InputProps) {
  const { t } = useTranslation()
  const id = useId()
  if (field.valueType === "boolean") {
    return (
      <RadioRow
        label={t("metadata.filter.boolLabel", { name: field.name })}
        value={draft.bool}
        onChange={(bool) => onChange({ bool })}
        options={[
          { value: "any", label: t("metadata.filter.boolAny") },
          { value: "true", label: t("metadata.filter.boolTrue") },
          { value: "false", label: t("metadata.filter.boolFalse") },
        ]}
      />
    )
  }
  const toggle = (optionId: number, on: boolean) =>
    onChange({ optionIds: on ? [...draft.optionIds, optionId] : draft.optionIds.filter((x) => x !== optionId) })
  return (
    <div className="flex flex-col !items-start gap-1.5">
      {field.options.length === 0 ? (
        <p className="text-xs break-keep text-muted-foreground">{t("metadata.filter.noOptions")}</p>
      ) : (
        <ul aria-label={t("metadata.filter.optionsLabel", { name: field.name })} className="flex w-full flex-col gap-1">
          {field.options.map((o) => (
            <li key={o.optionId} className="flex items-start gap-2">
              <Checkbox
                id={`${id}-${o.optionId}`}
                className="mt-0.5"
                checked={draft.optionIds.includes(o.optionId)}
                onCheckedChange={(v) => toggle(o.optionId, v === true)}
              />
              <Label htmlFor={`${id}-${o.optionId}`} className="min-w-0 font-normal break-words">
                {o.label}
              </Label>
            </li>
          ))}
        </ul>
      )}
      {field.valueType === "multiEnum" && (
        <RadioRow
          label={t("metadata.filter.matchLabel", { name: field.name })}
          value={draft.all ? "all" : "any"}
          onChange={(v) => onChange({ all: v === "all" })}
          options={[
            { value: "any", label: t("metadata.filter.matchAny") },
            { value: "all", label: t("metadata.filter.matchAll") },
          ]}
        />
      )}
    </div>
  )
}
