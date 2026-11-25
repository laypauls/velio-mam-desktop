import { FolderSync, RefreshCw, Trash2 } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { useRelocateFolder, useRescanFolder } from "@/hooks/useFolderAdmin"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatDateTime } from "@/lib/format"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { RelocateFolderDialog, type RelocateTarget } from "../RelocateFolderDialog"
import { RemoveFolderDialog } from "../RemoveFolderDialog"

interface Props {
  folder: ManagedFolder
  /** 저장하지 않은 편집이 있으면 즉시 실행 작업을 막는다 (저장 안 된 값과 섞이는 혼동 방지) */
  dirty: boolean
}

/**
 * 경로 표시 + [폴더 변경] — OS 폴더 선택 뒤 기존 `RelocateFolderDialog` 를 위에 겹쳐 연다.
 * 변경 후 경로는 상위가 `useFolders` 의 최신 폴더를 넘겨 갱신된다.
 */
export function FolderPathRow({ folder, dirty }: Props) {
  const { t } = useTranslation()
  const api = useApi()
  const relocate = useRelocateFolder()
  const [target, setTarget] = useState<RelocateTarget | null>(null)

  const pickNewPath = async () => {
    try {
      const dir = await api.pickDirectory()
      if (dir !== null) setTarget({ folder, newPath: dir })
    } catch (e) {
      toast.error(t(errorKey(e)))
    }
  }

  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-sm font-medium">{t("settings.folders.dialog.path")}</span>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <span className="min-w-0 flex-1 text-sm break-all">{folder.path}</span>
        <Button type="button" variant="outline" size="sm" disabled={dirty || relocate.isPending} onClick={() => void pickNewPath()}>
          <FolderSync aria-hidden="true" />
          {t("settings.folders.dialog.relocate")}
        </Button>
      </div>
      <RelocateFolderDialog relocate={relocate} target={target} onClose={() => setTarget(null)} />
    </div>
  )
}

/**
 * 즉시 실행 작업 — 지금 재스캔(대화상자 유지)·폴더 제거(확인 다이얼로그를 위에 겹쳐 열기).
 * 제거가 성공하면 폴더가 목록에서 사라지므로 상위 다이얼로그가 스스로 닫힌다.
 */
export function FolderActionsSection({ folder, dirty }: Props) {
  const { t } = useTranslation()
  const rescan = useRescanFolder()
  const [removeTarget, setRemoveTarget] = useState<ManagedFolder | null>(null)

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={dirty || rescan.isPending || folder.availability === "offline"}
          onClick={() => rescan.mutate(folder.folderId)}
        >
          <RefreshCw aria-hidden="true" />
          {t("settings.folders.dialog.rescanNow")}
        </Button>
        <span className="text-xs text-muted-foreground break-keep">
          {t("settings.folders.dialog.lastScan", {
            scan: folder.lastFullScanAt === null ? t("settings.folders.neverScanned") : formatDateTime(folder.lastFullScanAt),
            state: t(`settings.folders.scanState.${folder.scanState}`),
          })}
        </span>
      </div>
      <div>
        <Button type="button" variant="outline" size="sm" className="text-destructive" disabled={dirty} onClick={() => setRemoveTarget(folder)}>
          <Trash2 aria-hidden="true" />
          {t("settings.folders.dialog.remove")}
        </Button>
      </div>
      {dirty && <p className="text-xs text-muted-foreground break-keep">{t("settings.folders.dialog.editingHint")}</p>}
      <RemoveFolderDialog folder={removeTarget} onClose={() => setRemoveTarget(null)} />
    </div>
  )
}
