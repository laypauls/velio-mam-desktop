import { RotateCcw } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { useClearResumeAssets, useResumeCount } from "@/hooks/useResume"
import type { AssetSummary } from "@/lib/ipc-contract"

/** 선택에 동영상·음성이 하나라도 있는가 — 메모리에 없는(모르는) 항목은 있을 수 있는 것으로 본다. 이미지만 선택됐으면 이어보기 항목을 보이지 않는다 */
export function includesPlayable(ids: readonly number[], lookup?: (assetId: number) => AssetSummary | undefined): boolean {
  if (!lookup) return true
  return ids.some((id) => {
    const a = lookup(id)
    return a === undefined || a.mediaType !== "image"
  })
}

/** 이어보기 초기화 동작 (D9) — 선택 중 기록이 하나도 없으면 비활성, 실행은 확인 없이 즉시(결과 건수 toast 는 `useClearResumeAssets`) */
function useResumeClear(ids: number[], playable: boolean) {
  const count = useResumeCount(ids, playable)
  const clear = useClearResumeAssets()
  return {
    disabled: !((count.data ?? 0) > 0) || clear.isPending,
    run: () => clear.mutate(ids),
  }
}

/** 컨텍스트 메뉴의 "이어보기 초기화" — 메뉴가 열릴 때(마운트) 기록 수를 조회한다 */
export function ContextMenuResumeItem({ ids, playable }: { ids: number[]; playable: boolean }) {
  const { t } = useTranslation()
  const { disabled, run } = useResumeClear(ids, playable)
  if (!playable) return null
  return (
    <ContextMenuItem disabled={disabled} onSelect={run}>
      <RotateCcw aria-hidden="true" />
      {t("context.resumeClear")}
    </ContextMenuItem>
  )
}

/** 선택 바의 "이어보기 초기화" 버튼 */
export function SelectionResumeButton({ ids, playable }: { ids: number[]; playable: boolean }) {
  const { t } = useTranslation()
  const { disabled, run } = useResumeClear(ids, playable)
  if (!playable) return null
  return (
    <Button size="sm" variant="outline" disabled={disabled} onClick={run}>
      <RotateCcw aria-hidden="true" />
      {t("selection.action.resumeClear")}
    </Button>
  )
}
