import { MessageSquareText } from "lucide-react"
import { useTranslation } from "react-i18next"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { useSubtitleActions } from "@/hooks/useSubtitleActions"
import { formatNumber } from "@/lib/format"
import type { AssetSummary } from "@/lib/ipc-contract"

/**
 * 컨텍스트 메뉴의 "자막 생성" — 없는 자막만 만든다(`force=false`, 다시 생성은 상세 패널에서).
 * 단일 선택은 동영상·음성일 때만 보이고, 다중 선택은 선택 전체를 대상으로 개수를 붙인다(이미지는 서버가 건너뜀).
 * 다중 선택이 모두 이미지로 확인된 경우(`playable=false`)는 보이지 않는다.
 */
export function ContextMenuSubtitleItem({ asset, ids, playable }: { asset: AssetSummary; ids: number[]; playable: boolean }) {
  const { t } = useTranslation()
  const { generate } = useSubtitleActions()
  if (ids.length === 1 ? asset.mediaType === "image" : !playable) return null
  return (
    <ContextMenuItem onSelect={() => void generate(ids)}>
      <MessageSquareText aria-hidden="true" />
      {ids.length > 1 ? t("context.subtitleMany", { count: formatNumber(ids.length) }) : t("context.subtitle")}
    </ContextMenuItem>
  )
}
