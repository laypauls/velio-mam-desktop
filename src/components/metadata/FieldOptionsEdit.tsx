import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useAddMetadataOption, useRenameMetadataOption, useReorderMetadataOptions } from "@/hooks/useMetadataFields"
import { errorKey, isCommandError } from "@/lib/errors"
import type { MetadataField, MetadataFieldOption } from "@/lib/ipc-contract-metadata"
import { DeleteOptionDialog } from "./DeleteMetadataDialogs"
import { nameKey } from "./metadataTypes"
import { OptionListEditor } from "./OptionListEditor"

/**
 * 저장된 필드의 선택지 편집 — 추가·이름 변경·순서·삭제가 각각 즉시 저장된다(삭제는 확인창을 거친다).
 * 이름 변경·순서 변경은 재색인이 필요 없다.
 */
export function FieldOptionsEdit({ field }: { field: MetadataField }) {
  const { t } = useTranslation()
  const add = useAddMetadataOption()
  const rename = useRenameMetadataOption()
  const reorder = useReorderMetadataOptions()
  const [addError, setAddError] = useState<string | null>(null)
  const [rowErrors, setRowErrors] = useState<ReadonlyMap<number, string>>(new Map())
  const [listError, setListError] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<MetadataFieldOption | null>(null)

  const messageOf = (e: unknown) => (isCommandError(e) && e.code === "already_registered" ? t("metadata.settings.option.duplicate") : t(errorKey(e)))
  const setRowError = (optionId: number, msg: string | null) =>
    setRowErrors((prev) => {
      const next = new Map(prev)
      if (msg === null) next.delete(optionId)
      else next.set(optionId, msg)
      return next
    })
  const taken = (label: string, exceptId: number | null) =>
    field.options.some((o) => o.optionId !== exceptId && nameKey(o.label) === nameKey(label))

  const onAdd = async (raw: string) => {
    const label = raw.trim()
    if (label === "" || taken(label, null)) {
      setAddError(t(label === "" ? "metadata.settings.option.required" : "metadata.settings.option.duplicate"))
      return false
    }
    try {
      await add.mutateAsync({ fieldId: field.fieldId, label })
      setAddError(null)
      return true
    } catch (e) {
      setAddError(messageOf(e))
      return false
    }
  }

  const onRename = async (optionId: number, raw: string) => {
    const label = raw.trim()
    if (label === "" || taken(label, optionId)) {
      setRowError(optionId, t(label === "" ? "metadata.settings.option.required" : "metadata.settings.option.duplicate"))
      return
    }
    if (field.options.find((o) => o.optionId === optionId)?.label === label) {
      setRowError(optionId, null)
      return
    }
    try {
      await rename.mutateAsync({ optionId, label })
      setRowError(optionId, null)
    } catch (e) {
      setRowError(optionId, messageOf(e))
    }
  }

  const onMove = async (optionId: number, delta: -1 | 1) => {
    const ids = field.options.map((o) => o.optionId)
    const from = ids.indexOf(optionId)
    const to = from + delta
    if (from < 0 || to < 0 || to >= ids.length) return
    ;[ids[from], ids[to]] = [ids[to], ids[from]]
    try {
      await reorder.mutateAsync({ fieldId: field.fieldId, optionIds: ids })
      setListError(null)
    } catch (e) {
      setListError(t(errorKey(e)))
    }
  }

  return (
    <>
      <OptionListEditor
        live={false}
        items={field.options.map((o) => ({ key: o.optionId, label: o.label }))}
        rowErrors={rowErrors}
        listError={listError}
        addError={addError}
        onClearAddError={() => setAddError(null)}
        onAdd={onAdd}
        onRename={(key, label) => void onRename(key, label)}
        onMove={(key, delta) => void onMove(key, delta)}
        onDelete={(key) => setDeleting(field.options.find((o) => o.optionId === key) ?? null)}
      />
      <DeleteOptionDialog option={deleting} onClose={() => setDeleting(null)} />
    </>
  )
}
