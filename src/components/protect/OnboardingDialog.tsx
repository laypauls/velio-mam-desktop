import { FolderPlus } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog"
import { AddFolderDialog } from "@/components/folders/AddFolderDialog"
import { useAddFolder } from "@/hooks/useAddFolder"
import { useFolders } from "@/hooks/useLibrary"
import { useSettings, useUpdateSettings } from "@/hooks/useSettings"
import { errorKey } from "@/lib/errors"
import { PasswordForm } from "./PasswordForm"

type Step = "folders" | "password"

function FoldersStep() {
  const { t } = useTranslation()
  const add = useAddFolder()
  const folders = useFolders()
  return (
    <div className="flex flex-col gap-3">
      <Button variant="outline" className="self-start" disabled={add.isPending} onClick={() => add.start()}>
        <FolderPlus aria-hidden="true" />
        {t("onboarding.addFolder")}
      </Button>
      {(folders.data?.length ?? 0) > 0 ? (
        <ul aria-label={t("onboarding.added")} className="flex flex-col divide-y divide-border rounded-lg border border-border">
          {folders.data?.map((f) => (
            <li key={f.folderId} className="px-3 py-1.5 text-sm break-all">
              {f.path}
            </li>
          ))}
        </ul>
      ) : (
        <p className="text-sm text-muted-foreground break-keep">{t("onboarding.noFolders")}</p>
      )}
      <AddFolderDialog flow={add} />
    </div>
  )
}

/** D12: 최초 실행 안내 — 관리 폴더 추가 → (선택) 보호 비밀번호 → 완료. 닫거나 건너뛰어도 다시 묻지 않는다. */
export function OnboardingDialog() {
  const { t } = useTranslation()
  const settings = useSettings()
  const update = useUpdateSettings({ silent: true })
  const [step, setStep] = useState<Step>("folders")
  const [passwordSet, setPasswordSet] = useState(false)

  const open = settings.data !== undefined && !settings.data.firstRunCompleted
  const finish = () => update.mutate({ firstRunCompleted: true })

  return (
    <Dialog open={open} onOpenChange={(o) => !o && finish()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t(`onboarding.${step}.title`)}</DialogTitle>
          <DialogDescription className="break-keep">{t(`onboarding.${step}.description`)}</DialogDescription>
        </DialogHeader>
        <p className="text-xs text-muted-foreground" aria-live="polite">
          {t("onboarding.step", { current: step === "folders" ? 1 : 2, total: 2 })}
        </p>
        {step === "folders" ? (
          <FoldersStep />
        ) : passwordSet ? (
          <p className="text-sm break-keep">{t("onboarding.password.done")}</p>
        ) : (
          <PasswordForm mode="set" onDone={() => setPasswordSet(true)} />
        )}
        {update.isError && (
          <p role="alert" className="text-xs text-destructive">
            {t(errorKey(update.error))}
          </p>
        )}
        <DialogFooter>
          {step === "folders" ? (
            <>
              <Button variant="ghost" onClick={finish}>
                {t("onboarding.skip")}
              </Button>
              <Button onClick={() => setStep("password")}>{t("onboarding.next")}</Button>
            </>
          ) : (
            <>
              <Button variant="ghost" onClick={() => setStep("folders")}>
                {t("onboarding.back")}
              </Button>
              <Button onClick={finish}>{t(passwordSet ? "onboarding.finish" : "onboarding.finishLater")}</Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
