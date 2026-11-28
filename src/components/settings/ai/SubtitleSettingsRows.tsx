import { MessageSquareText } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
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
import { Switch } from "@/components/ui/switch"
import { qk } from "@/hooks/keys"
import { useSubtitleActions } from "@/hooks/useSubtitleActions"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import { effectiveAutoSubtitle } from "@/lib/ipc-contract-ai"
import { useQueryClient } from "@tanstack/react-query"
import { SettingRow } from "../SettingRow"
import type { AiCtl } from "./types"

/** 자막 일괄 생성 — 확인 뒤 자막 없는 동영상·음성 전체를 등록한다. 결과는 toast 로 알린다. */
function BatchRow({ blockedReason }: { blockedReason: string | null }) {
  const { t } = useTranslation()
  const api = useApi()
  const qc = useQueryClient()
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState(false)

  const run = async () => {
    setBusy(true)
    try {
      const { queued } = await api.generateSubtitlesAll()
      if (queued > 0) toast.success(t("ai.subtitle.queued", { queued: formatNumber(queued) }))
      else toast.info(t("ai.subtitle.nothingQueued"))
      void qc.invalidateQueries({ queryKey: qk.assetAiAll })
    } catch (e) {
      toast.error(t(errorKey(e)))
    } finally {
      setBusy(false)
    }
  }

  return (
    <>
      <SettingRow label={t("ai.local.subtitleBatch")} description={blockedReason ?? t("ai.local.subtitleBatchHint")}>
        <Button size="sm" variant="outline" disabled={blockedReason !== null || busy} onClick={() => setOpen(true)}>
          <MessageSquareText aria-hidden="true" />
          {t("ai.local.subtitleBatchButton")}
        </Button>
      </SettingRow>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("ai.local.subtitleBatchConfirmTitle")}</AlertDialogTitle>
            <AlertDialogDescription className="break-keep">{t("ai.local.subtitleBatchConfirmDescription")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void run()}>{t("ai.local.subtitleBatchStart")}</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

/** 설정 › 로컬 AI·검색의 자막 하위 행 — 새 파일 자동 자막 생성(실효값 표시)·자막 일괄 생성. 자막이 꺼져 있거나 모델이 없으면 둘 다 비활성. */
export function SubtitleSettingsRows({ ctl }: { ctl: AiCtl }) {
  const { t } = useTranslation()
  const { settings, patch } = ctl
  const { block } = useSubtitleActions()
  const blockedReason = block === "disabled" ? t("ai.local.subtitleOffReason") : block === "modelMissing" ? t("ai.local.subtitleModelMissingReason") : null
  return (
    <div className="flex flex-col gap-3 border-l border-border pl-4">
      <SettingRow label={t("ai.local.subtitleAutoNew")} description={blockedReason ?? t("ai.local.subtitleAutoNewHint")}>
        <Switch
          aria-label={t("ai.local.subtitleAutoNew")}
          checked={effectiveAutoSubtitle(settings)}
          disabled={blockedReason !== null}
          onCheckedChange={(autoSubtitleNew) => patch({ autoSubtitleNew })}
        />
      </SettingRow>
      <BatchRow blockedReason={blockedReason} />
    </div>
  )
}
