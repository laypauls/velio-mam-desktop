import { useState, type ReactElement } from "react"
import { AssetMenuBody } from "@/components/gallery/AssetMenuBody"
import { ReanalyzeDialog } from "@/components/gallery/ContextMenuAiItem"
import { TitleDialog } from "@/components/gallery/TitleDialog"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import type { AssetDetail, AssetSummary } from "@/lib/ipc-contract"

/**
 * 콘텐츠 보기의 미디어 스테이지(이미지·`<video>`·`<audio>`) 우클릭 메뉴 — 갤러리 메뉴 항목(`AssetMenuBody`)을 그대로 쓰되
 * 대상은 지금 보는 자산 하나이고 이미 열려 있으므로 "미리보기"는 숨긴다.
 * 컨트롤 바·재생 구간 레인 등은 감싸지 않는다(레인은 자체 메뉴가 있다). 제목/재분석 Dialog 는 메뉴 밖에 둔다.
 * 메뉴를 처음 열 때까지는 메뉴 본문·Dialog 를 마운트하지 않는다.
 */
export function ViewerContextMenu({ asset, children }: { asset: AssetDetail; children: ReactElement }) {
  const [used, setUsed] = useState(false)
  const [titleTarget, setTitleTarget] = useState<AssetSummary | null>(null)
  const [reanalyzeId, setReanalyzeId] = useState<number | null>(null)
  return (
    <>
      <ContextMenu onOpenChange={(open) => open && setUsed(true)}>
        <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
        {used && <AssetMenuBody hidePreview target={{ asset, ids: [asset.assetId] }} onEditTitle={setTitleTarget} onReanalyze={setReanalyzeId} />}
      </ContextMenu>
      {used && (
        <>
          <TitleDialog asset={titleTarget} onClose={() => setTitleTarget(null)} />
          <ReanalyzeDialog assetId={reanalyzeId} onClose={() => setReanalyzeId(null)} />
        </>
      )}
    </>
  )
}
