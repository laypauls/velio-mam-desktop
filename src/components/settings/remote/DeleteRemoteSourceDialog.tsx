import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import { useDeleteRemoteSource } from "@/hooks/useRemoteSources"
import { errorKey, isCommandError } from "@/lib/errors"
import type { RemoteSource } from "@/lib/ipc-contract"

interface Props {
  /** 삭제할 접속 (null = 닫힘) */
  source: RemoteSource | null
  onClose: () => void
}

/**
 * 접속 삭제 확인. 접속 정보(비밀 포함)만 지우고 원본 미디어는 건드리지 않는다.
 * 쓰는 폴더가 있으면 서버가 `busy` 로 거부하고, 이 다이얼로그는 닫히지 않은 채 그 사유를 인라인으로 보여 준다.
 */
export function DeleteRemoteSourceDialog({ source, onClose }: Props) {
  const { t } = useTranslation()
  const del = useDeleteRemoteSource()
  const [failure, setFailure] = useState<string | null>(null)

  const close = () => {
    setFailure(null)
    del.reset()
    onClose()
  }
  const confirm = async () => {
    if (source === null) return
    setFailure(null)
    try {
      await del.mutateAsync(source.sourceId)
      toast.success(t("settings.folders.remote.deleted", { name: source.name }))
      close()
    } catch (e) {
      setFailure(isCommandError(e) && e.code === "busy" ? "settings.folders.remote.deleteBusy" : errorKey(e))
    }
  }

  return (
    <AlertDialog open={source !== null} onOpenChange={(o) => !o && !del.isPending && close()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("settings.folders.remote.deleteTitle", { name: source?.name ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription className="break-keep">{t("settings.folders.remote.deleteDescription")}</AlertDialogDescription>
        </AlertDialogHeader>
        {failure !== null && (
          <p role="alert" className="text-sm text-destructive break-keep">
            {t(failure)}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={del.isPending}>{t("common.cancel")}</AlertDialogCancel>
          <Button variant="destructive" disabled={del.isPending} aria-busy={del.isPending} onClick={() => void confirm()}>
            {t("settings.folders.remote.delete")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
