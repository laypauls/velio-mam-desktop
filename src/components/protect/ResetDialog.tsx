import { useQuery } from "@tanstack/react-query"
import { useId, useState } from "react"
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
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { qk } from "@/hooks/keys"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { useProtectStore } from "@/stores"
import { PasswordInput } from "./PasswordInput"

function ResetForm({ onDone }: { onDone: () => void }) {
  const { t } = useTranslation()
  const api = useApi()
  const forgotId = useId()
  const status = useQuery({ queryKey: qk.protection, queryFn: () => api.getProtectionStatus() })
  const [password, setPassword] = useState("")
  const [forgot, setForgot] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const noPassword = status.data?.passwordSet === false
  const skipPassword = forgot || noPassword

  const submit = async () => {
    if (!skipPassword && password === "") {
      setError(t("protect.prompt.errorEmpty"))
      return
    }
    setBusy(true)
    try {
      await api.resetManagedData({ password: skipPassword ? null : password, forgotPassword: skipPassword })
      onDone()
    } catch (e) {
      setError(t(errorKey(e)))
      setBusy(false)
    }
  }

  return (
    <>
      <AlertDialogHeader>
        <AlertDialogTitle>{t("protect.reset.title")}</AlertDialogTitle>
        <AlertDialogDescription className="break-keep">{t("protect.reset.description")}</AlertDialogDescription>
      </AlertDialogHeader>
      <p className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-sm break-keep">{t("protect.reset.originalsSafe")}</p>
      {noPassword ? (
        <p className="text-sm text-muted-foreground break-keep">{t("protect.reset.noPassword")}</p>
      ) : (
        <div className="flex flex-col gap-3">
          <PasswordInput
            label={t("protect.password")}
            value={password}
            error={error}
            disabled={forgot}
            onChange={(e) => {
              setPassword(e.target.value)
              setError(null)
            }}
          />
          <div className="flex items-start gap-2">
            <Checkbox id={forgotId} checked={forgot} onCheckedChange={(v) => { setForgot(v === true); setError(null) }} />
            <div className="flex flex-col gap-0.5">
              <Label htmlFor={forgotId} className="!items-start">
                {t("protect.reset.forgot")}
              </Label>
              <p className="text-xs text-muted-foreground break-keep">{t("protect.reset.forgotHint")}</p>
            </div>
          </div>
        </div>
      )}
      {noPassword && error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
      <AlertDialogFooter>
        <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
        <Button variant="destructive" disabled={busy || status.isPending} onClick={() => void submit()}>
          {t("protect.reset.confirm")}
        </Button>
      </AlertDialogFooter>
    </>
  )
}

/** D11/PL3: 관리 데이터 초기화 — 경고 + 비밀번호 또는 "비밀번호를 잊었습니다" 경로, 완료 뒤 재시작 안내(차단). */
export function ResetDialog() {
  const { t } = useTranslation()
  const open = useProtectStore((s) => s.resetOpen)
  const setOpen = useProtectStore((s) => s.setResetOpen)
  const [done, setDone] = useState(false)
  return (
    <>
      <AlertDialog open={open && !done} onOpenChange={setOpen}>
        <AlertDialogContent>{open && <ResetForm onDone={() => setDone(true)} />}</AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={done}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("protect.reset.doneTitle")}</AlertDialogTitle>
            <AlertDialogDescription className="break-keep">{t("protect.reset.doneDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
