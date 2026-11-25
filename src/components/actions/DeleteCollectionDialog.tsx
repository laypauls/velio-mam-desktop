import { useTranslation } from "react-i18next"
import { toast } from "sonner"
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
import { useDeleteCollection, useToastError } from "@/hooks/useUserData"
import { useActionsStore } from "@/stores"

/** 컬렉션 삭제 확인 — 컬렉션만 지우고 미디어 파일은 지우지 않음을 명시한다. */
export function DeleteCollectionDialog() {
  const { t } = useTranslation()
  const target = useActionsStore((s) => s.deleteCollection)
  const ask = useActionsStore((s) => s.askDeleteCollection)
  const del = useDeleteCollection()
  const toastError = useToastError()

  const confirm = () => {
    if (!target) return
    del.mutate(target.collectionId, {
      onSuccess: () => toast.success(t("collections.deleted", { name: target.name })),
      onError: toastError,
    })
  }

  return (
    <AlertDialog open={target !== null} onOpenChange={(o) => !o && ask(null)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("collections.deleteTitle", { name: target?.name ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription className="break-keep">{t("collections.deleteDescription")}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction variant="destructive" disabled={del.isPending} onClick={confirm}>
            {t("collections.deleteConfirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
