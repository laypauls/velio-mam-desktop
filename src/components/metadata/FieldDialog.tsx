import { useId, useRef, useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { useCreateMetadataField, useRenameMetadataField } from "@/hooks/useMetadataFields"
import { errorKey, isCommandError } from "@/lib/errors"
import type { MetadataField, MetadataValueType } from "@/lib/ipc-contract-metadata"
import { FieldOptionsEdit } from "./FieldOptionsEdit"
import { hasOptions, METADATA_NAME_MAX, METADATA_VALUE_TYPES, nameKey, useMetadataTypeLabel } from "./metadataTypes"
import { OptionListEditor, type OptionItem } from "./OptionListEditor"

/** 대화 상자 대상 — 새 필드(create) 또는 저장된 필드(edit, 목록의 최신 상태를 따라간다) */
export type FieldDialogTarget = { kind: "create" } | { kind: "edit"; fieldId: number }

interface Props {
  target: FieldDialogTarget | null
  fields: readonly MetadataField[]
  onClose: () => void
}

function FieldForm({ edit, fields, onClose }: { edit: MetadataField | null; fields: readonly MetadataField[]; onClose: () => void }) {
  const { t } = useTranslation()
  const typeLabel = useMetadataTypeLabel()
  const create = useCreateMetadataField()
  const rename = useRenameMetadataField()
  const mutation = edit ? rename : create
  const nameId = useId()
  const nameErrorId = useId()
  const typeId = useId()
  const [name, setName] = useState(edit?.name ?? "")
  const [type, setType] = useState<MetadataValueType>(edit?.valueType ?? "keyword")
  const [draft, setDraft] = useState<OptionItem[]>([])
  const [addError, setAddError] = useState<string | null>(null)
  const [submitted, setSubmitted] = useState(false)
  const nextKey = useRef(1)

  const trimmed = name.trim()
  const duplicate = trimmed !== "" && fields.some((f) => f.fieldId !== edit?.fieldId && nameKey(f.name) === nameKey(trimmed))
  const backendDuplicate = mutation.isError && isCommandError(mutation.error) && mutation.error.code === "already_registered"
  const nameError =
    trimmed === "" && submitted
      ? t("metadata.settings.dialog.nameRequired")
      : duplicate || backendDuplicate
        ? t("metadata.settings.dialog.nameDuplicate")
        : mutation.isError
          ? t(errorKey(mutation.error))
          : null

  // 생성 초안 검증 — 제출을 한 번 시도한 뒤부터 행 오류를 보인다
  const draftErrors = new Map<number, string>()
  for (const item of draft) {
    if (item.label.trim() === "") draftErrors.set(item.key, t("metadata.settings.option.required"))
    else if (draft.some((o) => o.key < item.key && nameKey(o.label) === nameKey(item.label))) draftErrors.set(item.key, t("metadata.settings.option.duplicate"))
  }
  const needOptions = !edit && hasOptions(type) && draft.length === 0
  const listError = submitted && needOptions ? t("metadata.settings.option.needOne") : null
  const draftInvalid = hasOptions(type) && (needOptions || draftErrors.size > 0)

  const onAddDraft = (raw: string) => {
    const label = raw.trim()
    if (label === "" || draft.some((o) => nameKey(o.label) === nameKey(label))) {
      setAddError(t(label === "" ? "metadata.settings.option.required" : "metadata.settings.option.duplicate"))
      return false
    }
    setAddError(null)
    setDraft((d) => [...d, { key: nextKey.current++, label }])
    return true
  }
  const onMoveDraft = (key: number, delta: -1 | 1) =>
    setDraft((d) => {
      const from = d.findIndex((o) => o.key === key)
      const to = from + delta
      if (from < 0 || to < 0 || to >= d.length) return d
      const next = [...d]
      ;[next[from], next[to]] = [next[to], next[from]]
      return next
    })

  const submit = (e: FormEvent) => {
    e.preventDefault()
    if (mutation.isPending) return
    setSubmitted(true)
    if (trimmed === "" || duplicate) return
    if (edit) {
      if (trimmed === edit.name) return onClose()
      rename.mutate({ fieldId: edit.fieldId, name: trimmed }, { onSuccess: onClose })
      return
    }
    if (draftInvalid) return
    create.mutate({ name: trimmed, valueType: type, options: hasOptions(type) ? draft.map((o) => o.label.trim()) : [] }, { onSuccess: onClose })
  }

  return (
    <form onSubmit={submit} noValidate className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <label htmlFor={nameId} className="text-sm font-medium">
          {t("metadata.settings.dialog.nameLabel")}
        </label>
        <Input
          id={nameId}
          value={name}
          autoFocus
          autoComplete="off"
          maxLength={METADATA_NAME_MAX}
          placeholder={t("metadata.settings.dialog.namePlaceholder")}
          aria-invalid={nameError !== null}
          aria-describedby={nameError !== null ? nameErrorId : undefined}
          onChange={(e) => {
            setName(e.target.value)
            if (mutation.isError) mutation.reset()
          }}
        />
        {nameError !== null && (
          <p id={nameErrorId} role="alert" className="text-xs text-destructive break-keep">
            {nameError}
          </p>
        )}
      </div>
      <div className="flex flex-col gap-1.5">
        <label htmlFor={typeId} className="text-sm font-medium">
          {t("metadata.settings.dialog.typeLabel")}
        </label>
        <Select value={type} disabled={edit !== null} onValueChange={(v) => setType(v as MetadataValueType)}>
          <SelectTrigger id={typeId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {METADATA_VALUE_TYPES.map((v) => (
              <SelectItem key={v} value={v}>
                {typeLabel(v)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <p className="text-xs text-muted-foreground break-keep">{t(edit ? "metadata.settings.dialog.typeLocked" : "metadata.settings.dialog.typeHint")}</p>
      </div>
      {hasOptions(type) && (
        <div className="flex flex-col gap-1.5">
          <span className="text-sm font-medium">{t("metadata.settings.dialog.optionsLabel")}</span>
          {edit ? (
            <>
              <p className="text-xs text-muted-foreground break-keep">{t("metadata.settings.dialog.optionsSavedHint")}</p>
              <FieldOptionsEdit field={edit} />
            </>
          ) : (
            <OptionListEditor
              live
              items={draft}
              rowErrors={submitted ? draftErrors : new Map()}
              listError={listError}
              addError={addError}
              onClearAddError={() => setAddError(null)}
              onAdd={onAddDraft}
              onRename={(key, label) => setDraft((d) => d.map((o) => (o.key === key ? { ...o, label } : o)))}
              onMove={onMoveDraft}
              onDelete={(key) => setDraft((d) => d.filter((o) => o.key !== key))}
            />
          )}
        </div>
      )}
      <DialogFooter>
        <Button type="button" variant="outline" disabled={mutation.isPending} onClick={onClose}>
          {t(edit ? "common.close" : "common.cancel")}
        </Button>
        <Button type="submit" aria-busy={mutation.isPending} disabled={mutation.isPending}>
          {t(edit ? "common.save" : "metadata.settings.dialog.create")}
        </Button>
      </DialogFooter>
    </form>
  )
}

/** 필드 추가/편집 — 이름 · 타입(편집 시 변경 불가) · 선택지(선택·다중 선택). 오류는 입력 옆에 인라인으로 보인다. */
export function FieldDialog({ target, fields, onClose }: Props) {
  const { t } = useTranslation()
  const edit = target?.kind === "edit" ? (fields.find((f) => f.fieldId === target.fieldId) ?? null) : null
  const open = target?.kind === "create" || edit !== null
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t(edit ? "metadata.settings.dialog.editTitle" : "metadata.settings.dialog.createTitle")}</DialogTitle>
          <DialogDescription className="break-keep">{t(edit ? "metadata.settings.dialog.editDescription" : "metadata.settings.dialog.createDescription")}</DialogDescription>
        </DialogHeader>
        {/* key 로 대상마다 입력·오류 상태를 새로 시작한다 */}
        {open && <FieldForm key={edit?.fieldId ?? "new"} edit={edit} fields={fields} onClose={onClose} />}
      </DialogContent>
    </Dialog>
  )
}
