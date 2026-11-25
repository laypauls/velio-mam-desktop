import { Check, Info, Loader2 } from "lucide-react"
import { useTranslation } from "react-i18next"
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
import { useFolderRemovalImpact, useRemoveFolder } from "@/hooks/useFolderAdmin"
import { errorKey, isCommandError } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { FolderRemovalImpact, ManagedFolder } from "@/lib/ipc-contract"

type ImpactQuery = ReturnType<typeof useFolderRemovalImpact>

/** 이미 제거됐거나 제거 중인 폴더 — 개수 조회가 `not_found` 로 거부된다 (제거 버튼 비활성) */
const isNotFound = (e: unknown) => isCommandError(e) && e.code === "not_found"

function ImpactList({ impact }: { impact: FolderRemovalImpact }) {
  const { t } = useTranslation()
  return (
    <ul className="flex list-disc flex-col gap-0.5 pl-5">
      <li>{t("settings.folders.impact.media", { count: formatNumber(impact.mediaCount) })}</li>
      <li>
        {t("settings.folders.impact.userData", {
          tags: formatNumber(impact.tagLinks),
          notes: formatNumber(impact.notes),
          collections: formatNumber(impact.collectionItems),
        })}
      </li>
      <li>{t("settings.folders.impact.ai", { count: formatNumber(impact.aiResults) })}</li>
    </ul>
  )
}

function RemoveBody({ impact, error }: { impact: ImpactQuery; error: unknown }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col gap-3 text-sm text-foreground break-keep">
      <p className="flex items-center gap-1.5 font-medium">
        <Check aria-hidden="true" className="size-4 shrink-0 text-primary" />
        {t("settings.folders.removeSafe")}
      </p>
      <div className="flex flex-col gap-1">
        {impact.isPending ? (
          <p role="status" aria-busy="true" className="flex items-center gap-1.5 text-muted-foreground">
            <Loader2 aria-hidden="true" className="size-4 animate-spin" />
            {t("settings.folders.impact.loading")}
          </p>
        ) : isNotFound(impact.error) ? (
          <p role="alert" className="text-destructive">
            {t(errorKey(impact.error))}
          </p>
        ) : impact.isError ? (
          <p className="text-muted-foreground">{t("settings.folders.impact.unknown")}</p>
        ) : (
          <>
            <p>{t("settings.folders.impact.lead")}</p>
            <ImpactList impact={impact.data} />
          </>
        )}
      </div>
      <p className="text-muted-foreground">{t("settings.folders.removeDescription")}</p>
      <p className="flex items-start gap-1.5 text-xs text-muted-foreground">
        <Info aria-hidden="true" className="mt-0.5 size-3.5 shrink-0" />
        {t("settings.folders.removeHint")}
      </p>
      {error !== null && (
        <p role="alert" className="text-xs text-destructive">
          {t(errorKey(error))}
        </p>
      )}
    </div>
  )
}

/** 관리 폴더 제거 확인 — 지워질 연결 데이터 개수를 보여 준다. 백업 실패 등 오류는 Dialog 안에 인라인으로 두고 열어 둔다. */
export function RemoveFolderDialog({ folder, onClose }: { folder: ManagedFolder | null; onClose: () => void }) {
  const { t } = useTranslation()
  const remove = useRemoveFolder()
  const impact = useFolderRemovalImpact(folder?.folderId ?? null)
  const close = () => {
    remove.reset()
    onClose()
  }
  return (
    <AlertDialog open={folder !== null} onOpenChange={(o) => !o && !remove.isPending && close()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("settings.folders.removeTitle", { path: folder?.path ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription asChild>
            <div>{folder && <RemoveBody key={folder.folderId} impact={impact} error={remove.isError ? remove.error : null} />}</div>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={remove.isPending}>{t("common.cancel")}</AlertDialogCancel>
          <Button
            variant="destructive"
            aria-busy={remove.isPending}
            disabled={remove.isPending || folder === null || isNotFound(impact.error)}
            onClick={() => folder && remove.mutate(folder.folderId, { onSuccess: close })}
          >
            {t("settings.folders.removeConfirm")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
