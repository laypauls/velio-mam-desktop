import { ArrowDown, ArrowUp, Plus, Trash2 } from "lucide-react"
import { useEffect, useId, useState, type KeyboardEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { METADATA_NAME_MAX } from "./metadataTypes"

export interface OptionItem {
  key: number
  label: string
}

interface Props {
  items: readonly OptionItem[]
  /** true = 입력할 때마다 이름을 반영(생성 초안), false = 입력 칸을 벗어나거나 Enter 일 때 반영(저장된 선택지는 즉시 저장되므로) */
  live: boolean
  disabled?: boolean
  /** 행별 오류 문구 (key → 문구) */
  rowErrors: ReadonlyMap<number, string>
  /** 목록 아래 오류(선택지 필요 등) — 추가 칸과 연결된다 */
  listError?: string | null
  /** 추가 — 성공하면 true (입력 칸을 비운다). 실패 사유는 `addError` 로 돌려준다 */
  onAdd: (label: string) => boolean | Promise<boolean>
  addError: string | null
  onClearAddError: () => void
  onRename: (key: number, label: string) => void
  onMove: (key: number, delta: -1 | 1) => void
  onDelete: (key: number) => void
}

function OptionRow({
  item,
  index,
  count,
  live,
  disabled,
  error,
  onRename,
  onMove,
  onDelete,
}: {
  item: OptionItem
  index: number
  count: number
  live: boolean
  disabled: boolean
  error: string | undefined
  onRename: Props["onRename"]
  onMove: Props["onMove"]
  onDelete: Props["onDelete"]
}) {
  const { t } = useTranslation()
  const errorId = useId()
  const [text, setText] = useState(item.label)
  useEffect(() => setText(item.label), [item.label])
  const commit = () => {
    if (text !== item.label) onRename(item.key, text)
  }
  const name = item.label.trim() === "" ? t("metadata.settings.option.unnamed", { n: index + 1 }) : item.label
  return (
    <li className="flex flex-col gap-1">
      <div className="flex items-center gap-1">
        <Input
          value={text}
          disabled={disabled}
          maxLength={METADATA_NAME_MAX}
          autoComplete="off"
          aria-label={t("metadata.settings.option.nameLabel", { n: index + 1 })}
          aria-invalid={error !== undefined}
          aria-describedby={error !== undefined ? errorId : undefined}
          className="h-8 min-w-0 flex-1"
          onChange={(e) => {
            setText(e.target.value)
            if (live) onRename(item.key, e.target.value)
          }}
          onBlur={live ? undefined : commit}
          onKeyDown={(e: KeyboardEvent<HTMLInputElement>) => {
            if (!live && e.key === "Enter") {
              e.preventDefault()
              commit()
            }
          }}
        />
        <Button type="button" size="icon-sm" variant="ghost" disabled={disabled || index === 0} aria-label={t("metadata.settings.option.moveUp", { name })} onClick={() => onMove(item.key, -1)}>
          <ArrowUp aria-hidden="true" />
        </Button>
        <Button type="button" size="icon-sm" variant="ghost" disabled={disabled || index === count - 1} aria-label={t("metadata.settings.option.moveDown", { name })} onClick={() => onMove(item.key, 1)}>
          <ArrowDown aria-hidden="true" />
        </Button>
        <Button type="button" size="icon-sm" variant="ghost" disabled={disabled} aria-label={t("metadata.settings.option.delete", { name })} onClick={() => onDelete(item.key)}>
          <Trash2 aria-hidden="true" className="text-destructive" />
        </Button>
      </div>
      {error !== undefined && (
        <p id={errorId} role="alert" className="text-xs text-destructive break-keep">
          {error}
        </p>
      )}
    </li>
  )
}

/** 선택(enum)·다중 선택(multiEnum) 필드의 선택지 목록 편집기 — 추가·이름 변경·순서(위/아래 버튼)·삭제. 저장 방식은 호출부가 정한다. */
export function OptionListEditor({ items, live, disabled = false, rowErrors, listError, onAdd, addError, onClearAddError, onRename, onMove, onDelete }: Props) {
  const { t } = useTranslation()
  const addId = useId()
  const addErrorId = useId()
  const [draft, setDraft] = useState("")
  const invalid = addError !== null || (listError ?? null) !== null
  const describedBy = [addError !== null ? addErrorId : null, listError ? `${addErrorId}-list` : null].filter(Boolean).join(" ")

  const add = async () => {
    if (await onAdd(draft)) setDraft("")
  }

  return (
    <div className="flex flex-col gap-2">
      {items.length === 0 ? (
        <p className="text-xs text-muted-foreground break-keep">{t("metadata.settings.option.empty")}</p>
      ) : (
        // relative: 스크롤 영역 안의 absolute 자손이 바깥 조상 기준으로 배치되지 않게 한다
        <ul aria-label={t("metadata.settings.option.listLabel")} className="relative flex max-h-60 flex-col gap-1.5 overflow-y-auto pr-1">
          {items.map((item, i) => (
            <OptionRow
              key={item.key}
              item={item}
              index={i}
              count={items.length}
              live={live}
              disabled={disabled}
              error={rowErrors.get(item.key)}
              onRename={onRename}
              onMove={onMove}
              onDelete={onDelete}
            />
          ))}
        </ul>
      )}
      <div className="flex items-center gap-1">
        <Input
          id={addId}
          value={draft}
          disabled={disabled}
          maxLength={METADATA_NAME_MAX}
          autoComplete="off"
          placeholder={t("metadata.settings.option.addPlaceholder")}
          aria-label={t("metadata.settings.option.addLabel")}
          aria-invalid={invalid}
          aria-describedby={describedBy || undefined}
          className="h-8 min-w-0 flex-1"
          onChange={(e) => {
            setDraft(e.target.value)
            if (addError !== null) onClearAddError()
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault()
              void add()
            }
          }}
        />
        <Button type="button" size="sm" variant="outline" disabled={disabled} onClick={() => void add()}>
          <Plus aria-hidden="true" />
          {t("metadata.settings.option.add")}
        </Button>
      </div>
      {addError !== null && (
        <p id={addErrorId} role="alert" className="text-xs text-destructive break-keep">
          {addError}
        </p>
      )}
      {listError && (
        <p id={`${addErrorId}-list`} role="alert" className="text-xs text-destructive break-keep">
          {listError}
        </p>
      )}
    </div>
  )
}
