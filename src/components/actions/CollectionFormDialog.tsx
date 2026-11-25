import { useId, useState, type FormEvent } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useCreateCollection, useRenameCollection } from "@/hooks/useUserData"
import { errorKey } from "@/lib/errors"
import { useActionsStore, type CollectionForm } from "@/stores"

function CollectionFormBody({ form }: { form: CollectionForm }) {
  const { t } = useTranslation()
  const close = useActionsStore((s) => s.closeCollectionForm)
  const create = useCreateCollection()
  const rename = useRenameCollection()
  const inputId = useId()
  const errorId = useId()
  const [name, setName] = useState(form.mode === "rename" ? form.collection.name : "")
  const [error, setError] = useState<string | null>(null)
  const pending = create.isPending || rename.isPending

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault()
    const value = name.trim()
    if (value === "") {
      setError(t("collections.errorEmpty"))
      return
    }
    try {
      if (form.mode === "create") {
        await create.mutateAsync({ name: value, assetIds: form.assetIds })
        toast.success(t("collections.created", { name: value }))
      } else {
        await rename.mutateAsync({ collectionId: form.collection.collectionId, name: value })
        toast.success(t("collections.renamed", { name: value }))
      }
      close()
    } catch (err) {
      setError(t(errorKey(err)))
    }
  }

  return (
    <form onSubmit={(e) => void onSubmit(e)} noValidate className="flex flex-col gap-4">
      <DialogHeader>
        <DialogTitle>{form.mode === "create" ? t("collections.createTitle") : t("collections.renameTitle")}</DialogTitle>
        <DialogDescription>{t("collections.formDescription")}</DialogDescription>
      </DialogHeader>
      <div className="flex flex-col gap-1.5">
        <Label htmlFor={inputId} className="!items-start">
          {t("collections.nameLabel")}
        </Label>
        <Input
          id={inputId}
          autoFocus
          value={name}
          maxLength={100}
          aria-invalid={error !== null}
          aria-describedby={error !== null ? errorId : undefined}
          onChange={(e) => {
            setName(e.target.value)
            setError(null)
          }}
        />
        {error !== null && (
          <p id={errorId} role="alert" className="text-xs break-keep text-destructive">
            {error}
          </p>
        )}
      </div>
      <DialogFooter>
        <Button type="button" variant="outline" onClick={close}>
          {t("common.cancel")}
        </Button>
        <Button type="submit" disabled={pending}>
          {form.mode === "create" ? t("collections.create") : t("common.save")}
        </Button>
      </DialogFooter>
    </form>
  )
}

/** 컬렉션 새로 만들기 / 이름 변경. 빈 이름은 command 호출 없이 인라인 오류. */
export function CollectionFormDialog() {
  const form = useActionsStore((s) => s.collectionForm)
  const close = useActionsStore((s) => s.closeCollectionForm)
  return (
    <Dialog open={form !== null} onOpenChange={(o) => !o && close()}>
      <DialogContent className="sm:max-w-md">
        {form !== null && <CollectionFormBody key={form.mode === "rename" ? form.collection.collectionId : "create"} form={form} />}
      </DialogContent>
    </Dialog>
  )
}
