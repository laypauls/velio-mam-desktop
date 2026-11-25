import { X } from "lucide-react"
import { useEffect, useId, useState, type KeyboardEvent, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { DateInput } from "@/components/common/DateInput"
import { parseDateText } from "@/components/common/dateText"
import { Button } from "@/components/ui/button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { MAX_KEYWORD_CHARS, type MetadataField, type MetadataValue } from "@/lib/ipc-contract-metadata"
import { parseText, sameValue, valueToText } from "./metadataValueCodec"

/** Radix SelectItem 은 빈 문자열 값을 허용하지 않는다 — "값 없음 / 변경 안 함" 항목의 sentinel */
export const NONE_SENTINEL = "__none__"

interface Props {
  field: MetadataField
  /** 현재 값 (단일 편집: 저장된 값, 대량 지정: 이번에 지정한 값 — 없으면 null) */
  value: MetadataValue | null
  /** 값을 확정했을 때 — `null` 은 값을 비웠다는 뜻. 글 입력은 포커스를 잃거나 Enter, 선택형은 고르는 즉시 */
  onCommit: (next: MetadataValue | null) => void
  /** 비어 있는 선택 항목의 이름 ("값 없음" / "변경 안 함") */
  emptyLabel: string
  /** 입력 칸 옆 버튼 (지우기·취소 등) */
  actions?: ReactNode
  /** 저장 실패 등 부모가 알리는 오류 */
  serverError?: string | null
}

/** 사용자 메타데이터 필드 한 칸의 입력 — 타입별 컨트롤. 단일 상세·대량 지정이 함께 쓴다. */
export function FieldValueInput({ field, value, onCommit, emptyLabel, actions, serverError = null }: Props) {
  const { t } = useTranslation()
  const inputId = useId()
  const errorId = useId()
  const labelId = useId()
  const type = field.valueType
  const external = type === "keyword" || type === "number" || type === "date" ? valueToText(value) : ""
  const [draft, setDraft] = useState(external)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setDraft(external)
    setError(null)
  }, [external])

  const commitText = (text: string = draft) => {
    if (type !== "keyword" && type !== "number" && type !== "date") return
    if (type === "date" && text !== "" && parseDateText(text) === undefined) {
      setError(t("metadata.detail.error.dateInvalid"))
      return
    }
    const r = parseText(type, text)
    if (!r.ok) {
      setError(t(`metadata.detail.error.${r.error}`, { max: MAX_KEYWORD_CHARS }))
      return
    }
    setError(null)
    if (!sameValue(r.value, value)) onCommit(r.value)
  }

  const shownError = error ?? serverError
  const describedBy = shownError !== null ? errorId : undefined

  let control: ReactNode
  if (type === "keyword" || type === "number" || type === "date") {
    const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Enter") e.currentTarget.blur()
    }
    control = type === "date" ? (
      <DateInput
        id={inputId}
        value={draft}
        aria-invalid={shownError !== null}
        aria-describedby={describedBy}
        onValueChange={(text, source) => {
          setDraft(text)
          setError(null)
          if (source === "picked") commitText(text)
        }}
        onBlur={() => commitText()}
        onKeyDown={onKeyDown}
      />
    ) : (
      <Input
        id={inputId}
        type="text"
        inputMode={type === "number" ? "decimal" : undefined}
        value={draft}
        aria-invalid={shownError !== null}
        aria-describedby={describedBy}
        onChange={(e) => {
          setDraft(e.target.value)
          setError(null)
        }}
        onBlur={() => commitText()}
        onKeyDown={onKeyDown}
      />
    )
  } else if (type === "multiEnum") {
    control = (
      <MultiEnumChecks
        field={field}
        labelId={labelId}
        value={value?.kind === "multiEnum" ? value.value : []}
        onChange={(ids) => onCommit(ids.length === 0 ? null : { kind: "multiEnum", value: ids })}
        describedBy={describedBy}
        invalid={shownError !== null}
      />
    )
  } else {
    const current =
      value === null ? NONE_SENTINEL : value.kind === "boolean" ? String(value.value) : value.kind === "enum" ? String(value.value) : NONE_SENTINEL
    const onSelect = (v: string) => {
      if (v === NONE_SENTINEL) onCommit(null)
      else if (type === "boolean") onCommit({ kind: "boolean", value: v === "true" })
      else onCommit({ kind: "enum", value: Number(v) })
    }
    control = (
      <Select value={current} onValueChange={onSelect}>
        <SelectTrigger id={inputId} className="w-full" aria-invalid={shownError !== null} aria-describedby={describedBy}>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={NONE_SENTINEL}>{emptyLabel}</SelectItem>
          {type === "boolean" ? (
            <>
              <SelectItem value="true">{t("metadata.detail.yes")}</SelectItem>
              <SelectItem value="false">{t("metadata.detail.no")}</SelectItem>
            </>
          ) : (
            field.options.map((o) => (
              <SelectItem key={o.optionId} value={String(o.optionId)}>
                {o.label}
              </SelectItem>
            ))
          )}
        </SelectContent>
      </Select>
    )
  }

  return (
    <div className="flex flex-col gap-1">
      {type === "multiEnum" ? (
        <span id={labelId} className="text-sm font-medium break-all">
          {field.name}
        </span>
      ) : (
        <Label htmlFor={inputId} className="text-sm break-all">
          {field.name}
        </Label>
      )}
      <div className="flex items-start gap-1">
        <div className="min-w-0 flex-1">{control}</div>
        {actions}
      </div>
      {shownError !== null && (
        <p id={errorId} role="alert" className="text-xs break-keep text-destructive">
          {shownError}
        </p>
      )}
    </div>
  )
}

function MultiEnumChecks({
  field,
  labelId,
  value,
  onChange,
  describedBy,
  invalid,
}: {
  field: MetadataField
  labelId: string
  value: number[]
  onChange: (ids: number[]) => void
  describedBy: string | undefined
  invalid: boolean
}) {
  const { t } = useTranslation()
  const key = value.join(",")
  const [selected, setSelected] = useState<number[]>(value)
  // 연속으로 눌러도 저장 응답 전의 선택이 사라지지 않게 로컬 선택을 기준으로 하고, 서버 값이 바뀌면 맞춘다
  useEffect(() => {
    setSelected(value)
  }, [key])

  if (field.options.length === 0) {
    return <p className="text-xs break-keep text-muted-foreground">{t("metadata.detail.noOptions")}</p>
  }
  const toggle = (optionId: number, on: boolean) => {
    const next = field.options.map((o) => o.optionId).filter((id) => (id === optionId ? on : selected.includes(id)))
    setSelected(next)
    onChange(next)
  }
  return (
    <div
      role="group"
      aria-labelledby={labelId}
      aria-invalid={invalid}
      aria-describedby={describedBy}
      className="flex max-h-40 flex-col gap-1.5 overflow-y-auto rounded-md border border-input p-2"
    >
      {field.options.map((o) => (
        <label key={o.optionId} className="flex items-center gap-2 text-sm">
          <Checkbox checked={selected.includes(o.optionId)} onCheckedChange={(v) => toggle(o.optionId, v === true)} />
          <span className="min-w-0 break-all">{o.label}</span>
        </label>
      ))}
    </div>
  )
}

/** 값 지우기 아이콘 버튼 — 날짜·다중 선택처럼 입력 칸을 비우기 어려운 타입에 쓴다 */
export function ClearValueButton({ name, onClick }: { name: string; onClick: () => void }) {
  const { t } = useTranslation()
  return (
    <Button type="button" size="icon-sm" variant="ghost" aria-label={t("metadata.detail.clearValue", { name })} onClick={onClick}>
      <X aria-hidden="true" />
    </Button>
  )
}
