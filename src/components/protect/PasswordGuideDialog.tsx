import { useTranslation } from "react-i18next"
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
import { useProtectStore } from "@/stores"

/** 비밀번호가 없어 보호 작업을 할 수 없을 때: 보호 탭으로 안내한다. */
export function PasswordGuideDialog() {
  const { t } = useTranslation()
  const open = useProtectStore((s) => s.passwordGuideOpen)
  const setOpen = useProtectStore((s) => s.setPasswordGuideOpen)
  const openSettings = useProtectStore((s) => s.openSettings)
  return (
    <AlertDialog open={open} onOpenChange={setOpen}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("protect.guide.title")}</AlertDialogTitle>
          <AlertDialogDescription className="break-keep">{t("protect.guide.description")}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.close")}</AlertDialogCancel>
          <AlertDialogAction onClick={() => openSettings("protection")}>{t("protect.guide.open")}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
