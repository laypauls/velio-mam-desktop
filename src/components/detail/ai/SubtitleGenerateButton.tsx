import { MessageSquareText, RefreshCw } from "lucide-react"
import { useState } from "react"
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
import { Button } from "@/components/ui/button"
import { useSubtitleActions } from "@/hooks/useSubtitleActions"
import { ReasonTooltip } from "../ReasonTooltip"

interface Props {
  assetId: number
  /** 자막이 이미 있다 — 있으면 [다시 생성](덮어쓰기 확인), 없으면 [자막 생성] */
  hasSubtitle: boolean
  /** 이 자산의 자막 단계가 대기·실행 중이다 */
  busy: boolean
}

/** 상세 패널 자막 영역의 생성 버튼. 다시 생성은 기존 자막을 덮어쓰므로 확인(AlertDialog) 뒤 `force=true` 로 요청한다. */
export function SubtitleGenerateButton({ assetId, hasSubtitle, busy }: Props) {
  const { t } = useTranslation()
  const { generate, reasonText } = useSubtitleActions()
  const [confirmOpen, setConfirmOpen] = useState(false)
  const reason = busy ? t("ai.subtitle.busy") : reasonText

  if (!hasSubtitle) {
    return (
      <ReasonTooltip reason={reason}>
        <Button size="sm" disabled={reason !== null} onClick={() => void generate([assetId], false)}>
          <MessageSquareText aria-hidden="true" />
          {t("ai.subtitle.generate")}
        </Button>
      </ReasonTooltip>
    )
  }
  return (
    <>
      <ReasonTooltip reason={reason}>
        <Button size="xs" variant="outline" disabled={reason !== null} onClick={() => setConfirmOpen(true)}>
          <RefreshCw aria-hidden="true" />
          {t("ai.subtitle.regenerate")}
        </Button>
      </ReasonTooltip>
      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("ai.subtitle.regenerateTitle")}</AlertDialogTitle>
            <AlertDialogDescription className="break-keep">{t("ai.subtitle.regenerateWarning")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void generate([assetId], true)}>{t("ai.subtitle.regenerateConfirm")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
