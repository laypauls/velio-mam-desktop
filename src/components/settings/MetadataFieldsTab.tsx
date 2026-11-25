import { ArrowDown, ArrowUp, Info, ListPlus, MoreHorizontal, Pencil, Plus, Tags, Trash2 } from "lucide-react"
import { useState } from "react"
import { toast } from "sonner"
import { useTranslation } from "react-i18next"
import { DeleteFieldDialog } from "@/components/metadata/DeleteMetadataDialogs"
import { FieldDialog, type FieldDialogTarget } from "@/components/metadata/FieldDialog"
import { hasOptions, useMetadataTypeLabel } from "@/components/metadata/metadataTypes"
import { Button } from "@/components/ui/button"
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table"
import { useMetadataFields, useReorderMetadataFields } from "@/hooks/useMetadataFields"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import { MAX_METADATA_FIELDS, type MetadataField } from "@/lib/ipc-contract-metadata"

function FieldRow({
  field,
  index,
  count,
  onMove,
  onEdit,
  onDelete,
}: {
  field: MetadataField
  index: number
  count: number
  onMove: (delta: -1 | 1) => void
  onEdit: () => void
  onDelete: () => void
}) {
  const { t } = useTranslation()
  const typeLabel = useMetadataTypeLabel()
  return (
    <TableRow>
      <TableCell className="w-16 px-1">
        <div className="flex items-center">
          <Button size="icon-xs" variant="ghost" disabled={index === 0} aria-label={t("metadata.settings.moveUp", { name: field.name })} onClick={() => onMove(-1)}>
            <ArrowUp aria-hidden="true" />
          </Button>
          <Button size="icon-xs" variant="ghost" disabled={index === count - 1} aria-label={t("metadata.settings.moveDown", { name: field.name })} onClick={() => onMove(1)}>
            <ArrowDown aria-hidden="true" />
          </Button>
        </div>
      </TableCell>
      <TableCell className="min-w-0 font-medium whitespace-normal break-all">{field.name}</TableCell>
      <TableCell className="w-28 whitespace-normal break-keep">{typeLabel(field.valueType)}</TableCell>
      <TableCell className="w-20 whitespace-normal break-keep text-muted-foreground">
        {hasOptions(field.valueType) ? t("metadata.settings.optionCount", { count: formatNumber(field.options.length) }) : t("metadata.settings.noOptions")}
      </TableCell>
      <TableCell className="w-10 px-1">
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button size="icon-sm" variant="ghost" aria-label={t("metadata.settings.rowMenu", { name: field.name })}>
              <MoreHorizontal aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={onEdit}>
              <Pencil aria-hidden="true" />
              {t("metadata.settings.edit")}
            </DropdownMenuItem>
            <DropdownMenuSeparator />
            <DropdownMenuItem variant="destructive" onSelect={onDelete}>
              <Trash2 aria-hidden="true" />
              {t("metadata.settings.delete")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </TableCell>
    </TableRow>
  )
}

/** 설정 > 메타데이터 필드: 사용자 정의 필드 목록(순서 변경·편집·삭제)과 추가. 필드는 최대 20개. */
export function MetadataFieldsTab() {
  const { t } = useTranslation()
  const fields = useMetadataFields()
  const reorder = useReorderMetadataFields()
  const [dialog, setDialog] = useState<FieldDialogTarget | null>(null)
  const [deleting, setDeleting] = useState<MetadataField | null>(null)
  const list = fields.data ?? []
  const full = list.length >= MAX_METADATA_FIELDS

  const move = (index: number, delta: -1 | 1) => {
    const ids = list.map((f) => f.fieldId)
    const to = index + delta
    if (to < 0 || to >= ids.length) return
    ;[ids[index], ids[to]] = [ids[to], ids[index]]
    reorder.mutate(ids, { onError: (e) => void toast.error(t(errorKey(e))) })
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <h3 className="text-sm font-semibold">{t("metadata.settings.title")}</h3>
        <span className="text-xs text-muted-foreground">{t("metadata.settings.usage", { count: formatNumber(list.length), max: MAX_METADATA_FIELDS })}</span>
        <Button size="sm" variant="outline" className="ml-auto" disabled={full || fields.isPending} aria-describedby={full ? "metadata-full-note" : undefined} onClick={() => setDialog({ kind: "create" })}>
          <Plus aria-hidden="true" />
          {t("metadata.settings.add")}
        </Button>
      </div>
      {full && (
        <p id="metadata-full-note" className="text-xs text-muted-foreground break-keep">
          {t("metadata.settings.limitReached", { max: MAX_METADATA_FIELDS })}
        </p>
      )}
      {fields.isError ? (
        <div role="alert" className="flex flex-col items-start gap-2 text-sm text-destructive">
          <p>{t("metadata.settings.loadError")}</p>
          <Button size="sm" variant="outline" onClick={() => void fields.refetch()}>
            {t("common.retry")}
          </Button>
        </div>
      ) : fields.isPending ? (
        <p role="status" aria-busy="true" className="text-sm text-muted-foreground">
          {t("common.loading")}
        </p>
      ) : list.length === 0 ? (
        <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border px-6 py-10 text-center">
          <Tags aria-hidden="true" className="size-8 text-muted-foreground" />
          <p className="text-sm font-semibold">{t("metadata.settings.emptyTitle")}</p>
          <p className="max-w-md text-xs text-muted-foreground break-keep">{t("metadata.settings.emptyDescription")}</p>
          <Button size="sm" onClick={() => setDialog({ kind: "create" })}>
            <ListPlus aria-hidden="true" />
            {t("metadata.settings.add")}
          </Button>
        </div>
      ) : (
        <div className="rounded-lg border border-border">
          <Table aria-label={t("metadata.settings.title")} className="table-fixed">
            <TableHeader>
              <TableRow>
                <TableHead className="w-16">
                  <span className="sr-only">{t("metadata.settings.col.order")}</span>
                </TableHead>
                <TableHead>{t("metadata.settings.col.name")}</TableHead>
                <TableHead className="w-28">{t("metadata.settings.col.type")}</TableHead>
                <TableHead className="w-20">{t("metadata.settings.col.options")}</TableHead>
                <TableHead className="w-10">
                  <span className="sr-only">{t("metadata.settings.col.actions")}</span>
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {list.map((f, i) => (
                <FieldRow
                  key={f.fieldId}
                  field={f}
                  index={i}
                  count={list.length}
                  onMove={(d) => move(i, d)}
                  onEdit={() => setDialog({ kind: "edit", fieldId: f.fieldId })}
                  onDelete={() => setDeleting(f)}
                />
              ))}
            </TableBody>
          </Table>
        </div>
      )}
      <p className="flex items-start gap-1.5 text-xs text-muted-foreground break-keep">
        <Info aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
        {t("metadata.settings.note")}
      </p>
      <FieldDialog target={dialog} fields={list} onClose={() => setDialog(null)} />
      <DeleteFieldDialog field={deleting} onClose={() => setDeleting(null)} />
    </div>
  )
}
