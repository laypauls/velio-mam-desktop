import { RefreshCw, Sparkles } from "lucide-react"
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
import { ContextMenuItem } from "@/components/ui/context-menu"
import { useAnalyzeActions } from "@/hooks/useAnalyze"
import type { AssetSummary } from "@/lib/ipc-contract"
import { useAiStore } from "@/stores"

interface ItemProps {
  asset: AssetSummary
  /** 다중 선택 안에서 우클릭했으면 선택 전체, 아니면 해당 자산 하나 */
  ids: number[]
  /** 메뉴가 닫혀도 유지되는 위치(`AssetContextMenu`)에서 재분석 확인 Dialog 를 연다 */
  onReanalyze: (assetId: number) => void
}

/** 컨텍스트 메뉴의 AI 항목: 다중=일괄 분석 Dialog, 단일 none/failed=분석, 단일 done=재분석(확인 필요). */
export function ContextMenuAiItem({ asset, ids, onReanalyze }: ItemProps) {
  const { t } = useTranslation()
  const { analyze } = useAnalyzeActions()

  if (ids.length > 1) {
    return (
      <ContextMenuItem onSelect={() => useAiStore.getState().openBulkAnalyze(ids)}>
        <Sparkles aria-hidden="true" />
        {t("context.analyzeMany")}
      </ContextMenuItem>
    )
  }
  if (asset.aiState === "done") {
    return (
      <ContextMenuItem onSelect={() => onReanalyze(asset.assetId)}>
        <RefreshCw aria-hidden="true" />
        {t("context.reanalyze")}
      </ContextMenuItem>
    )
  }
  return (
    <ContextMenuItem onSelect={() => void analyze([asset.assetId], null, false)}>
      <Sparkles aria-hidden="true" />
      {t("context.analyze")}
    </ContextMenuItem>
  )
}

/** 재분석(덮어쓰기) 확인. 메뉴 밖에 두어 메뉴가 닫혀도 남는다. */
export function ReanalyzeDialog({ assetId, onClose }: { assetId: number | null; onClose: () => void }) {
  const { t } = useTranslation()
  const { analyze } = useAnalyzeActions()

  return (
    <AlertDialog open={assetId !== null} onOpenChange={(open) => !open && onClose()}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("ai.reanalyze.title")}</AlertDialogTitle>
          <AlertDialogDescription className="break-keep">{t("ai.overwriteWarning")}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction onClick={() => assetId !== null && void analyze([assetId], null, true)}>
            {t("ai.reanalyze.confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
