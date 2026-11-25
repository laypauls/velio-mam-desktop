import { Scissors } from "lucide-react"
import { useTranslation } from "react-i18next"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { useSceneDetect } from "@/hooks/useSceneCuts"
import type { AssetSummary } from "@/lib/ipc-contract"

/**
 * 컨텍스트 메뉴의 "장면전환점 추출" — 선택한 동영상을 높은 우선순위로 대기열에 올린다 (이미 추출된 영상은 서버가 건너뛴다).
 * 단일 선택은 영상일 때만, 다중 선택은 영상이 섞여 있을 수 있어 항상 보이고 영상이 아닌 항목은 서버가 건너뛴다.
 */
export function ContextMenuSceneItem({ asset, ids }: { asset: AssetSummary; ids: number[] }) {
  const { t } = useTranslation()
  const detect = useSceneDetect()
  if (ids.length === 1 && asset.mediaType !== "video") return null
  return (
    <ContextMenuItem onSelect={() => detect.mutate({ assetIds: ids, options: {} })}>
      <Scissors aria-hidden="true" />
      {t("context.sceneExtract")}
    </ContextMenuItem>
  )
}
