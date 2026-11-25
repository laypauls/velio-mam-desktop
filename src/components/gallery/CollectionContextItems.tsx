import { ArrowDownToLine, ArrowUp, ArrowDown, ArrowUpToLine, ArrowUpDown, FolderMinus, Play } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import {
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu"
import { useStartPlayback } from "@/hooks/usePlayQueue"
import { useRemoveFromCollection, useToastError } from "@/hooks/useUserData"
import { formatNumber } from "@/lib/format"
import { useSearchStore } from "@/stores"
import type { ReorderOp } from "./reorderTarget"
import { useReorder } from "./useReorderActions"

const ORDER_ITEMS: { op: ReorderOp; icon: typeof ArrowUp; shortcut: string }[] = [
  { op: "start", icon: ArrowUpToLine, shortcut: "Ctrl+Shift+Home" },
  { op: "up", icon: ArrowUp, shortcut: "Ctrl+Shift+↑" },
  { op: "down", icon: ArrowDown, shortcut: "Ctrl+Shift+↓" },
  { op: "end", icon: ArrowDownToLine, shortcut: "Ctrl+Shift+End" },
]

/** 컬렉션 보기 우클릭 메뉴의 "여기부터 재생" — 컬렉션 순서 전체를 이 항목부터 연속 재생한다. */
export function CollectionPlayItem({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const start = useStartPlayback()
  if (view.kind !== "collection") return null
  return (
    <ContextMenuItem onSelect={() => void start(view.collectionId, assetId)}>
      <Play aria-hidden="true" />
      {t("context.playFrom")}
    </ContextMenuItem>
  )
}

/** 컬렉션 보기 우클릭 메뉴의 "순서 이동 ▸" 와 "현재 컬렉션에서 제거". 순서 이동은 컬렉션 순서 정렬일 때만 켜진다. */
export function CollectionOrderItems({ ids }: { ids: number[] }) {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const reorder = useReorder()
  const remove = useRemoveFromCollection()
  const toastError = useToastError()
  if (view.kind !== "collection") return null
  const collectionId = view.collectionId
  return (
    <>
      <ContextMenuSub>
        <ContextMenuSubTrigger>
          <ArrowUpDown aria-hidden="true" />
          {t("order.move")}
        </ContextMenuSubTrigger>
        <ContextMenuSubContent>
          {!reorder.enabled && <ContextMenuLabel className="max-w-56 break-keep">{t("order.onlyInCollectionOrder")}</ContextMenuLabel>}
          {ORDER_ITEMS.map(({ op, icon: Icon, shortcut }) => (
            <ContextMenuItem key={op} disabled={!reorder.enabled} onSelect={() => void reorder.run(op, ids)}>
              <Icon aria-hidden="true" />
              {t(`order.${op}`)}
              <ContextMenuShortcut>{shortcut}</ContextMenuShortcut>
            </ContextMenuItem>
          ))}
        </ContextMenuSubContent>
      </ContextMenuSub>
      <ContextMenuItem
        onSelect={() =>
          remove.mutate(
            { collectionId, assetIds: ids },
            { onSuccess: () => toast.success(t("collections.removed", { count: formatNumber(ids.length) })), onError: toastError }
          )
        }
      >
        <FolderMinus aria-hidden="true" />
        {t("collections.removeFromCurrent")}
      </ContextMenuItem>
    </>
  )
}
