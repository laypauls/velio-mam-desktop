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
import { useClearResumeScope } from "@/hooks/useResume"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"

/** 초기화 확인 대상 — 폴더(표시 이름·폴더 id) 또는 전체. `count` 는 확인 시점의 기록 수 */
export type ResumeResetTarget = { kind: "folder"; folderId: number; name: string; count: number } | { kind: "all"; count: number }

/**
 * 이어보기 기록 초기화 확인 (D9) — 원본 파일은 건드리지 않으므로 비밀번호 인증은 없다.
 * 실패는 Dialog 안에 인라인으로 두고 열어 둔다. 성공하면 건수 toast 와 기록 수 갱신은 `useClearResumeScope` 가 한다.
 */
export function ResumeResetDialog({ target, onClose }: { target: ResumeResetTarget | null; onClose: () => void }) {
  const { t } = useTranslation()
  const clear = useClearResumeScope()
  const close = () => {
    clear.reset()
    onClose()
  }
  const count = formatNumber(target?.count ?? 0)
  const body = target === null ? "" : target.kind === "folder" ? t("settings.resume.confirmFolder", { name: target.name, count }) : t("settings.resume.confirmAll", { count })
  return (
    <AlertDialog open={target !== null} onOpenChange={(o) => !o && !clear.isPending && close()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("settings.resume.confirmTitle")}</AlertDialogTitle>
          <AlertDialogDescription className="break-keep">
            {body} {t("settings.resume.unaffected")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {clear.isError && (
          <p role="alert" className="text-xs text-destructive">
            {t(errorKey(clear.error))}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={clear.isPending}>{t("common.cancel")}</AlertDialogCancel>
          <Button
            variant="destructive"
            aria-busy={clear.isPending}
            disabled={clear.isPending || target === null}
            onClick={() => target && clear.mutate(target.kind === "folder" ? { kind: "folder", folderId: target.folderId } : { kind: "all" }, { onSuccess: close })}
          >
            {t("settings.resume.reset")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
