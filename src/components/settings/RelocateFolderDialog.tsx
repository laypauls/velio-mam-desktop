import { useEffect } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import type { useRelocateFolder } from "@/hooks/useFolderAdmin"
import { errorKey } from "@/lib/errors"
import type { ManagedFolder } from "@/lib/ipc-contract"

export interface RelocateTarget {
  folder: ManagedFolder
  newPath: string
}

interface Props {
  /** 호출부가 소유한 mutation — 행 버튼의 진행 중 비활성화와 상태를 공유한다 */
  relocate: ReturnType<typeof useRelocateFolder>
  target: RelocateTarget | null
  onClose: () => void
}

/** 관리 폴더 경로 변경 확인 — 오류는 다이얼로그 안에 인라인으로 보이고 열린 채 유지한다. */
export function RelocateFolderDialog({ relocate, target, onClose }: Props) {
  const { t } = useTranslation()
  const { reset } = relocate

  // 다시 열 때(대상 변경) 이전 오류를 지운다
  useEffect(() => {
    reset()
  }, [target, reset])

  const confirm = () => {
    if (!target) return
    relocate.mutate({ folderId: target.folder.folderId, newPath: target.newPath }, { onSuccess: onClose })
  }

  return (
    <Dialog open={target !== null} onOpenChange={(o) => !o && !relocate.isPending && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("settings.folders.relocateTitle")}</DialogTitle>
          <DialogDescription className="break-keep">{t("settings.folders.relocateDescription")}</DialogDescription>
        </DialogHeader>
        <dl className="flex flex-col gap-3 text-sm">
          {target?.folder.alias && (
            <div className="flex flex-col gap-1">
              <dt className="text-xs text-muted-foreground">{t("settings.folders.relocateFolder")}</dt>
              <dd className="break-all">{target.folder.alias}</dd>
            </div>
          )}
          <div className="flex flex-col gap-1">
            <dt className="text-xs text-muted-foreground">{t("settings.folders.relocateCurrent")}</dt>
            <dd className="break-all">{target?.folder.path}</dd>
          </div>
          <div className="flex flex-col gap-1">
            <dt className="text-xs text-muted-foreground">{t("settings.folders.relocateNew")}</dt>
            <dd className="font-medium break-all">{target?.newPath}</dd>
          </div>
        </dl>
        {relocate.isError && (
          <p role="alert" className="text-xs text-destructive break-keep">
            {t(errorKey(relocate.error))}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" disabled={relocate.isPending} onClick={onClose}>
            {t("common.cancel")}
          </Button>
          <Button aria-busy={relocate.isPending} disabled={relocate.isPending} onClick={confirm}>
            {t("settings.folders.relocateConfirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
