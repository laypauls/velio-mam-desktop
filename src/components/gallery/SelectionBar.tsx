import { ArrowUpDown, ChevronDown, Copy, MessageSquareText, Sparkles, Download, FolderInput, FolderPlus, Heart, Star, Tag, Trash2, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { CollectionCheckItems, DROPDOWN_MENU_PRIMITIVES } from "@/components/collections/CollectionCheckItems"
import { ReasonTooltip } from "@/components/detail/ReasonTooltip"
import { RATING_MENU_VALUES, RatingMenuLabel } from "@/components/rating/RatingMenuLabel"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { useCopyToClipboard } from "@/hooks/useReuse"
import { useSettings } from "@/hooks/useSettings"
import { useSubtitleActions } from "@/hooks/useSubtitleActions"
import { useRemoveFromCollection, useSetFavorite, useSetRating, useToastError } from "@/hooks/useUserData"
import { formatNumber } from "@/lib/format"
import type { AssetSummary } from "@/lib/ipc-contract"
import { useActionsStore, useAiStore, useProtectStore, useSearchStore, useSelectionStore } from "@/stores"
import type { ReorderOp } from "./reorderTarget"
import { SelectionResumeButton, includesPlayable } from "./ResumeClearActions"
import { useReorder } from "./useReorderActions"

function FavoriteMenu({ ids }: { ids: number[] }) {
  const { t } = useTranslation()
  const setFavorite = useSetFavorite()
  const toastError = useToastError()
  const apply = (favorite: boolean) =>
    setFavorite.mutate(
      { assetIds: ids, favorite },
      {
        onSuccess: () => toast.success(t(favorite ? "favorite.added" : "favorite.removed", { count: formatNumber(ids.length) })),
        onError: toastError,
      }
    )
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="outline" disabled={setFavorite.isPending}>
          <Heart aria-hidden="true" />
          {t("selection.action.favorite")}
          <ChevronDown aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        <DropdownMenuItem onSelect={() => apply(true)}>{t("favorite.on")}</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => apply(false)}>{t("favorite.off")}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function RatingMenu({ ids }: { ids: number[] }) {
  const { t } = useTranslation()
  const setRating = useSetRating()
  const toastError = useToastError()
  const apply = (rating: number) =>
    setRating.mutate(
      { assetIds: ids, rating },
      {
        onSuccess: () => toast.success(t(rating === 0 ? "rating.cleared" : "rating.changed", { count: formatNumber(ids.length) })),
        onError: toastError,
      }
    )
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="outline" disabled={setRating.isPending}>
          <Star aria-hidden="true" />
          {t("rating.label")}
          <ChevronDown aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {RATING_MENU_VALUES.map((n) => (
          <DropdownMenuItem key={n} onSelect={() => apply(n)}>
            <RatingMenuLabel value={n} />
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem onSelect={() => apply(0)}>{t("rating.clear")}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function CollectionMenu({ ids }: { ids: number[] }) {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const remove = useRemoveFromCollection()
  const toastError = useToastError()
  const count = formatNumber(ids.length)

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="outline">
          <FolderPlus aria-hidden="true" />
          {t("selection.action.collection")}
          <ChevronDown aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="min-w-56">
        <CollectionCheckItems ids={ids} primitives={DROPDOWN_MENU_PRIMITIVES} />
        {view.kind === "collection" && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={() =>
                remove.mutate(
                  { collectionId: view.collectionId, assetIds: ids },
                  { onSuccess: () => toast.success(t("collections.removed", { count })), onError: toastError }
                )
              }
            >
              {t("collections.removeFromCurrent")}
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

const ORDER_OPS: ReorderOp[] = ["start", "up", "down", "end"]

/** 컬렉션 보기에서만: 선택 항목을 한 덩어리로 옮기는 "순서 이동" 메뉴 — 컬렉션 순서 정렬이 아니면 항목을 끄고 이유를 안내한다. */
function ReorderMenu({ ids }: { ids: number[] }) {
  const { t } = useTranslation()
  const reorder = useReorder()
  if (!reorder.inCollection) return null
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="outline">
          <ArrowUpDown aria-hidden="true" />
          {t("order.move")}
          <ChevronDown aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start">
        {!reorder.enabled && <DropdownMenuLabel className="max-w-56 break-keep">{t("order.onlyInCollectionOrder")}</DropdownMenuLabel>}
        {ORDER_OPS.map((op) => (
          <DropdownMenuItem key={op} disabled={!reorder.enabled} onSelect={() => void reorder.run(op, ids)}>
            {t(`order.${op}`)}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/** D9: 선택 2개 이상이면 일괄 작업 바. 삭제/이동은 확인 → 비밀번호(D10) 흐름으로 이어진다. */
export function SelectionBar({ lookup }: { lookup?: (assetId: number) => AssetSummary | undefined } = {}) {
  const { t } = useTranslation()
  const selected = useSelectionStore((s) => s.ids)
  const clear = useSelectionStore((s) => s.clear)
  const loadedOnly = useSelectionStore((s) => s.loadedOnly)
  const openBulkTags = useActionsStore((s) => s.openBulkTags)
  const openExport = useActionsStore((s) => s.openExport)
  const copy = useCopyToClipboard()
  const askOriginals = useProtectStore((s) => s.askOriginals)
  const openBulkAnalyze = useAiStore((s) => s.openBulkAnalyze)
  const originalActions = useSettings().data?.originalActionsEnabled ?? false
  const subtitle = useSubtitleActions()
  if (selected.size < 2) return null
  const ids = [...selected]
  const subtitleReason = !includesPlayable(ids, lookup) ? t("ai.subtitle.reason.imagesOnly") : subtitle.reasonText
  return (
    <div role="toolbar" aria-label={t("selection.bar")} className="flex flex-wrap items-center gap-2 border-b border-border bg-muted/40 px-4 py-2">
      <span className="text-sm font-medium break-keep">{t("selection.count", { count: formatNumber(selected.size) })}</span>
      {loadedOnly && <span className="text-xs text-muted-foreground break-keep">{t("selection.loadedOnly", { count: formatNumber(selected.size) })}</span>}
      <Button size="sm" variant="outline" onClick={() => openBulkTags(ids)}>
        <Tag aria-hidden="true" />
        {t("selection.action.tag")}
      </Button>
      <Button size="sm" variant="outline" onClick={() => openBulkAnalyze(ids)}>
        <Sparkles aria-hidden="true" />
        {t("selection.action.ai")}
      </Button>
      <ReasonTooltip reason={subtitleReason}>
        <Button size="sm" variant="outline" disabled={subtitleReason !== null} onClick={() => void subtitle.generate(ids)}>
          <MessageSquareText aria-hidden="true" />
          {t("selection.action.subtitle")}
        </Button>
      </ReasonTooltip>
      <FavoriteMenu ids={ids} />
      <RatingMenu ids={ids} />
      <CollectionMenu ids={ids} />
      <ReorderMenu ids={ids} />
      <Button size="sm" variant="outline" onClick={() => openExport(ids)}>
        <Download aria-hidden="true" />
        {t("selection.action.export")}
      </Button>
      <SelectionResumeButton ids={ids} playable={includesPlayable(ids, lookup)} />
      <Button size="sm" variant="outline" disabled={copy.isPending} onClick={() => copy.mutate(ids)}>
        <Copy aria-hidden="true" />
        {t("selection.action.copy")}
      </Button>
      {originalActions && (
        <>
          <Button size="sm" variant="outline" onClick={() => askOriginals({ kind: "move", assetIds: ids })}>
            <FolderInput aria-hidden="true" />
            {t("selection.action.move")}
          </Button>
          <Button size="sm" variant="outline" onClick={() => askOriginals({ kind: "delete", assetIds: ids })}>
            <Trash2 aria-hidden="true" className="text-destructive" />
            {t("selection.action.delete")}
          </Button>
        </>
      )}
      <Button size="icon-sm" variant="ghost" className="ml-auto" aria-label={t("selection.clear")} onClick={clear}>
        <X aria-hidden="true" />
      </Button>
    </div>
  )
}
