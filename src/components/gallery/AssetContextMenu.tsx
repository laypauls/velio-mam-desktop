import { useState, type MouseEvent, type ReactNode } from "react"
import { ContextMenu, ContextMenuTrigger } from "@/components/ui/context-menu"
import { cn } from "@/lib/utils"
import type { AssetSummary } from "@/lib/ipc-contract"
import { AssetMenuBody, type MenuTarget } from "./AssetMenuBody"
import { includesPlayable } from "./ResumeClearActions"
import { ReanalyzeDialog } from "./ContextMenuAiItem"
import type { ReorderDrag } from "./useReorderDrag"
import { TitleDialog } from "./TitleDialog"
import { useSelectionStore } from "@/stores"

/**
 * 갤러리·목록 항목의 컨텍스트 메뉴. 가상 스크롤 항목마다 메뉴를 만들지 않도록 목록 전체를 하나의 트리거로 감싸고,
 * 우클릭 시 `data-asset-id` 로 대상 항목을 `lookup`(메모리에 있는 블록에서 찾기)으로 얻는다. 항목 밖에서는 메뉴를 열지 않는다.
 */
export function AssetContextMenu({ lookup, className, drag, children }: { lookup: (assetId: number) => AssetSummary | undefined; className?: string; drag?: ReorderDrag; children: ReactNode }) {
  const [target, setTarget] = useState<MenuTarget | null>(null)
  const [titleTarget, setTitleTarget] = useState<AssetSummary | null>(null)
  const [reanalyzeId, setReanalyzeId] = useState<number | null>(null)

  const onContextMenuCapture = (e: MouseEvent<HTMLDivElement>) => {
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-asset-id]")
    const asset = el ? lookup(Number(el.dataset.assetId)) : undefined
    if (!asset) {
      e.preventDefault() // Radix 는 defaultPrevented 이면 메뉴를 열지 않는다
      return
    }
    const sel = useSelectionStore.getState()
    if (!sel.ids.has(asset.assetId)) sel.selectOnly(asset.assetId)
    const after = useSelectionStore.getState().ids
    const ids = after.size > 1 ? [...after] : [asset.assetId]
    setTarget({ asset, ids, hasPlayable: includesPlayable(ids, lookup) })
  }

  return (
    <>
      <ContextMenu>
        <ContextMenuTrigger asChild>
          <div className={cn("relative", className)} onContextMenuCapture={onContextMenuCapture} {...drag?.dragProps}>
            {children}
            {/* 순서 재배치 드래그의 놓을 위치 표시선 — 위치는 드래그 훅이 DOM 으로 직접 갱신한다 */}
            {drag && <div ref={drag.indicatorRef} hidden aria-hidden="true" className="pointer-events-none absolute z-20 rounded-full bg-primary" />}
          </div>
        </ContextMenuTrigger>
        {target !== null && <AssetMenuBody target={target} onEditTitle={setTitleTarget} onReanalyze={setReanalyzeId} />}
      </ContextMenu>
      <TitleDialog asset={titleTarget} onClose={() => setTitleTarget(null)} />
      <ReanalyzeDialog assetId={reanalyzeId} onClose={() => setReanalyzeId(null)} />
    </>
  )
}
