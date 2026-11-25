import { ChevronDown, RefreshCw, Sparkles } from "lucide-react"
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
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { useAnalyzeActions } from "@/hooks/useAnalyze"

interface Props {
  assetId: number
  /** 분석 결과가 이미 있다 — 있으면 [재분석 ▾], 없으면 [분석] */
  hasResult: boolean
  /** 사용자가 편집한 결과가 있다 — 재분석은 덮어쓰므로 확인을 받는다 */
  edited: boolean
  /** 대기·실행 중인 분석이 있다 */
  busy: boolean
}

/** D4: 결과 없음 → [분석]. 결과 있음 → [재분석](덮어쓰기) + ▾ "빈 항목만 분석". 편집한 결과가 있으면 재분석 전에 경고한다. */
export function AnalysisActions({ assetId, hasResult, edited, busy }: Props) {
  const { t } = useTranslation()
  const { analyze } = useAnalyzeActions()
  const [confirmOpen, setConfirmOpen] = useState(false)

  if (!hasResult) {
    return (
      <Button size="sm" disabled={busy} onClick={() => void analyze([assetId], null, false)}>
        <Sparkles aria-hidden="true" />
        {t("ai.action.analyze")}
      </Button>
    )
  }
  return (
    <div className="flex items-center">
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        className="rounded-r-none"
        onClick={() => (edited ? setConfirmOpen(true) : void analyze([assetId], null, true))}
      >
        <RefreshCw aria-hidden="true" />
        {t("ai.action.reanalyze")}
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            size="icon-sm"
            variant="outline"
            disabled={busy}
            aria-label={t("ai.action.more")}
            className="rounded-l-none border-l-0"
          >
            <ChevronDown aria-hidden="true" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => void analyze([assetId], null, false)}>
            {t("ai.action.fillMissing")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <AlertDialog open={confirmOpen} onOpenChange={setConfirmOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("ai.reanalyze.title")}</AlertDialogTitle>
            <AlertDialogDescription className="break-keep">{t("ai.overwriteWarning")}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void analyze([assetId], null, true)}>
              {t("ai.reanalyze.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
