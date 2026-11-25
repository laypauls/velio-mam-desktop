import { Pencil, Tags, Trash2 } from "lucide-react"
import { useId, useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { EmptyState } from "@/components/gallery/EmptyState"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { useTags } from "@/hooks/useLibrary"
import { useDeleteTag, useRenameTag, useToastError } from "@/hooks/useUserData"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { Tag } from "@/lib/ipc-contract"
import { useActionsStore } from "@/stores"

function RenameForm({ tag, onDone }: { tag: Tag; onDone: () => void }) {
  const { t } = useTranslation()
  const rename = useRenameTag()
  const errorId = useId()
  const [name, setName] = useState(tag.name)
  const [error, setError] = useState<string | null>(null)

  const submit = async (e: FormEvent) => {
    e.preventDefault()
    const value = name.trim()
    if (value === "") {
      setError(t("tags.errorEmpty"))
      return
    }
    try {
      await rename.mutateAsync({ tagId: tag.tagId, name: value })
      toast.success(t("tags.renamed", { name: value }))
      onDone()
    } catch (err) {
      setError(t(errorKey(err)))
    }
  }

  return (
    <form onSubmit={(e) => void submit(e)} noValidate className="flex min-w-0 flex-1 flex-col gap-1">
      <div className="flex items-center gap-2">
        <Input
          autoFocus
          value={name}
          maxLength={100}
          aria-label={t("tags.renameField", { name: tag.name })}
          aria-invalid={error !== null}
          aria-describedby={error !== null ? errorId : undefined}
          onChange={(e) => {
            setName(e.target.value)
            setError(null)
          }}
        />
        <Button type="submit" size="sm" disabled={rename.isPending}>
          {t("common.save")}
        </Button>
        <Button type="button" size="sm" variant="outline" onClick={onDone}>
          {t("common.cancel")}
        </Button>
      </div>
      {error !== null && (
        <p id={errorId} role="alert" className="text-xs break-keep text-destructive">
          {error}
        </p>
      )}
    </form>
  )
}

function TagRow({ tag, onDelete }: { tag: Tag; onDelete: (tag: Tag) => void }) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState(false)
  return (
    <li className="flex items-center gap-2 border-b border-border py-1.5 last:border-b-0">
      {editing ? (
        <RenameForm tag={tag} onDone={() => setEditing(false)} />
      ) : (
        <>
          <span className="min-w-0 flex-1 truncate text-sm">{tag.name}</span>
          <span className="text-xs text-muted-foreground tabular-nums">{t("tags.count", { count: formatNumber(tag.assetCount) })}</span>
          <Button size="icon-sm" variant="ghost" aria-label={t("tags.rename", { name: tag.name })} onClick={() => setEditing(true)}>
            <Pencil aria-hidden="true" />
          </Button>
          <Button size="icon-sm" variant="ghost" aria-label={t("tags.delete", { name: tag.name })} onClick={() => onDelete(tag)}>
            <Trash2 aria-hidden="true" className="text-destructive" />
          </Button>
        </>
      )}
    </li>
  )
}

/** 태그 관리 — 전체 태그와 사용 개수, 이름 변경, 삭제(AlertDialog). */
export function TagManagerDialog() {
  const { t } = useTranslation()
  const open = useActionsStore((s) => s.tagManagerOpen)
  const setOpen = useActionsStore((s) => s.setTagManagerOpen)
  const tags = useTags()
  const del = useDeleteTag()
  const toastError = useToastError()
  const [target, setTarget] = useState<Tag | null>(null)

  const confirmDelete = () => {
    if (!target) return
    del.mutate(target.tagId, {
      onSuccess: () => toast.success(t("tags.deleted", { name: target.name })),
      onError: toastError,
    })
  }

  return (
    <>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("tags.managerTitle")}</DialogTitle>
            <DialogDescription>{t("tags.managerDescription")}</DialogDescription>
          </DialogHeader>
          {tags.isError ? (
            <p role="alert" className="text-sm text-destructive">
              {t(errorKey(tags.error))}
            </p>
          ) : tags.data && tags.data.length === 0 ? (
            <EmptyState icon={Tags} title={t("tags.empty.title")} description={t("tags.empty.description")} />
          ) : (
            <ScrollArea className="h-72">
              <ul aria-label={t("tags.managerTitle")} className="pr-3">
                {(tags.data ?? []).map((tag) => (
                  <TagRow key={tag.tagId} tag={tag} onDelete={setTarget} />
                ))}
              </ul>
            </ScrollArea>
          )}
        </DialogContent>
      </Dialog>
      <AlertDialog open={target !== null} onOpenChange={(o) => !o && setTarget(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("tags.deleteTitle", { name: target?.name ?? "" })}</AlertDialogTitle>
            <AlertDialogDescription className="break-keep">
              {t("tags.deleteDescription", { count: formatNumber(target?.assetCount ?? 0) })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction variant="destructive" disabled={del.isPending} onClick={confirmDelete}>
              {t("tags.deleteConfirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
