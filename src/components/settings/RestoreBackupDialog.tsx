import { useQuery } from "@tanstack/react-query"
import { useState } from "react"
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
import { PasswordInput } from "@/components/protect/PasswordInput"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatDateTime } from "@/lib/format"
import type { BackupInfo } from "@/lib/ipc-contract"

function RestoreForm({ backup, onCancel }: { backup: BackupInfo; onCancel: () => void }) {
  const { t } = useTranslation()
  const api = useApi()
  const status = useQuery({ queryKey: qk.protection, queryFn: () => api.getProtectionStatus() })
  const [password, setPassword] = useState("")
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const needsPassword = status.data?.passwordSet === true

  const submit = async () => {
    if (needsPassword && password === "") {
      setError(t("protect.prompt.errorEmpty"))
      return
    }
    setBusy(true)
    try {
      // 성공하면 앱이 재시작된다 — 닫지 않고 busy 를 유지한다
      await api.restoreBackup(needsPassword ? { fileName: backup.fileName, password } : { fileName: backup.fileName })
    } catch (e) {
      setError(t(errorKey(e)))
      setBusy(false)
    }
  }

  return (
    <>
      <AlertDialogHeader>
        <AlertDialogTitle>{t("settings.backup.restore.title")}</AlertDialogTitle>
        <AlertDialogDescription className="break-keep">
          {t("settings.backup.restore.description", { date: formatDateTime(backup.createdAt) })}
        </AlertDialogDescription>
      </AlertDialogHeader>
      {needsPassword ? (
        <PasswordInput
          label={t("protect.password")}
          value={password}
          error={error}
          disabled={busy}
          onChange={(e) => {
            setPassword(e.target.value)
            setError(null)
          }}
        />
      ) : (
        error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )
      )}
      <AlertDialogFooter>
        <AlertDialogCancel disabled={busy} onClick={onCancel}>
          {t("common.cancel")}
        </AlertDialogCancel>
        <Button disabled={busy || status.isPending} aria-busy={busy} onClick={() => void submit()}>
          {t("settings.backup.restore.confirm")}
        </Button>
      </AlertDialogFooter>
    </>
  )
}

/** 백업 복원 확인 — 비밀번호가 설정돼 있을 때만 비밀번호 입력을 보인다(PR2). 성공하면 앱이 다시 시작된다. */
export function RestoreBackupDialog({ backup, onClose }: { backup: BackupInfo | null; onClose: () => void }) {
  return (
    <AlertDialog open={backup !== null} onOpenChange={(o) => !o && onClose()}>
      <AlertDialogContent>{backup && <RestoreForm backup={backup} onCancel={onClose} />}</AlertDialogContent>
    </AlertDialog>
  )
}
