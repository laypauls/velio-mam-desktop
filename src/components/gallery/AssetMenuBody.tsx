import { Download, ExternalLink, Eye, FolderInput, FolderOpen, FolderPlus, Heart, Pencil, Star, Tag, Trash2 } from "lucide-react"
import { useTranslation } from "react-i18next"
import { unavailableReason } from "@/components/detail/AssetActions"
import { RATING_MENU_VALUES, RatingMenuLabel } from "@/components/rating/RatingMenuLabel"
import {
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuShortcut,
  ContextMenuSub,
  ContextMenuSubContent,
  ContextMenuSubTrigger,
} from "@/components/ui/context-menu"
import { useSettings } from "@/hooks/useSettings"
import { useOriginalActions } from "@/hooks/useReuse"
import { useSetFavorite, useSetRating, useToastError } from "@/hooks/useUserData"
import type { AssetSummary } from "@/lib/ipc-contract"
import { CONTEXT_MENU_PRIMITIVES, CollectionCheckItems } from "@/components/collections/CollectionCheckItems"
import { CollectionOrderItems, CollectionPlayItem } from "./CollectionContextItems"
import { ContextMenuAiItem } from "./ContextMenuAiItem"
import { ContextMenuSubtitleItem } from "./ContextMenuSubtitleItem"
import { ContextMenuSceneItem } from "./ContextMenuSceneItem"
import { ContextMenuResumeItem } from "./ResumeClearActions"
import { PluginMenuSlot } from "./PluginMenuSlot"
import { useActionsStore, useProtectStore, useViewStore } from "@/stores"

export interface MenuTarget {
  asset: AssetSummary
  /** 다중 선택 안에서 우클릭했으면 선택 전체, 아니면 해당 자산 하나 */
  ids: number[]
  /** 선택에 동영상·음성이 있는가 — 생략하면 대상 자산의 형식으로 판단한다 (이어보기 초기화 항목 표시 조건) */
  hasPlayable?: boolean
}

/** 자산 컨텍스트 메뉴 항목 — 갤러리(`AssetContextMenu`)와 콘텐츠 보기(`ViewerContextMenu`)가 공유한다 */
export function AssetMenuBody({
  target,
  onEditTitle,
  onReanalyze,
  hidePreview = false,
}: {
  target: MenuTarget
  onEditTitle: (asset: AssetSummary) => void
  onReanalyze: (assetId: number) => void
  /** 이미 콘텐츠 보기에서 열려 있는 자산이면 "미리보기" 항목을 숨긴다 */
  hidePreview?: boolean
}) {
  const { t } = useTranslation()
  const { asset, ids } = target
  const openPreview = useViewStore((s) => s.openPreview)
  const openExport = useActionsStore((s) => s.openExport)
  const askOriginals = useProtectStore((s) => s.askOriginals)
  const original = useOriginalActions()
  const settings = useSettings().data
  const titleEditEnabled = settings?.titleEditEnabled ?? false
  const originalActions = settings?.originalActionsEnabled ?? false
  const setFavorite = useSetFavorite()
  const setRating = useSetRating()
  const toastError = useToastError()
  const reason = unavailableReason(asset.availability, t)

  return (
    <ContextMenuContent className="w-56">
      {!hidePreview && (
        <ContextMenuItem disabled={reason !== null} onSelect={() => openPreview(asset.assetId)}>
          <Eye aria-hidden="true" />
          {t("context.preview")}
          {reason !== null && <ContextMenuShortcut>{t(`asset.availability.${asset.availability}`)}</ContextMenuShortcut>}
        </ContextMenuItem>
      )}
      <CollectionPlayItem assetId={asset.assetId} />
      <ContextMenuItem disabled={reason !== null} onSelect={() => original.open(asset.assetId)}>
        <ExternalLink aria-hidden="true" />
        {t("detail.open")}
        {reason !== null && <ContextMenuShortcut>{t(`asset.availability.${asset.availability}`)}</ContextMenuShortcut>}
      </ContextMenuItem>
      <ContextMenuItem disabled={reason !== null} onSelect={() => original.reveal(asset.assetId)}>
        <FolderOpen aria-hidden="true" />
        {t("detail.reveal")}
        {reason !== null && <ContextMenuShortcut>{t(`asset.availability.${asset.availability}`)}</ContextMenuShortcut>}
      </ContextMenuItem>
      {titleEditEnabled && ids.length === 1 && (
        <ContextMenuItem onSelect={() => onEditTitle(asset)}>
          <Pencil aria-hidden="true" />
          {t("context.editTitle")}
        </ContextMenuItem>
      )}
      <ContextMenuSeparator />
      <ContextMenuItem
        onSelect={() => setFavorite.mutate({ assetIds: ids, favorite: !asset.favorite }, { onError: toastError })}
      >
        <Heart aria-hidden="true" />
        {asset.favorite ? t("favorite.off") : t("favorite.on")}
      </ContextMenuItem>
      <ContextMenuSub>
        <ContextMenuSubTrigger>
          <Star aria-hidden="true" />
          {t("rating.label")}
        </ContextMenuSubTrigger>
        <ContextMenuSubContent>
          {RATING_MENU_VALUES.map((n) => (
            <ContextMenuItem key={n} onSelect={() => setRating.mutate({ assetIds: ids, rating: n }, { onError: toastError })}>
              <RatingMenuLabel value={n} />
            </ContextMenuItem>
          ))}
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={() => setRating.mutate({ assetIds: ids, rating: 0 }, { onError: toastError })}>{t("rating.clear")}</ContextMenuItem>
        </ContextMenuSubContent>
      </ContextMenuSub>
      <ContextMenuSub>
        <ContextMenuSubTrigger>
          <FolderPlus aria-hidden="true" />
          {t("context.addToCollection")}
        </ContextMenuSubTrigger>
        <ContextMenuSubContent>
          <CollectionCheckItems ids={ids} primitives={CONTEXT_MENU_PRIMITIVES} />
        </ContextMenuSubContent>
      </ContextMenuSub>
      <CollectionOrderItems ids={ids} />
      <ContextMenuItem onSelect={() => useActionsStore.getState().openBulkTags(ids)}>
        <Tag aria-hidden="true" />
        {t("context.tags")}
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuAiItem asset={asset} ids={ids} onReanalyze={onReanalyze} />
      <ContextMenuSubtitleItem asset={asset} ids={ids} playable={target.hasPlayable ?? asset.mediaType !== "image"} />
      <ContextMenuSceneItem asset={asset} ids={ids} />
      <ContextMenuResumeItem ids={ids} playable={target.hasPlayable ?? asset.mediaType !== "image"} />
      <PluginMenuSlot asset={asset} ids={ids} />
      <ContextMenuItem onSelect={() => openExport(ids)}>
        <Download aria-hidden="true" />
        {t("selection.action.export")}
      </ContextMenuItem>
      {originalActions && (
        <>
          <ContextMenuSeparator />
          <ContextMenuItem disabled={reason !== null} onSelect={() => askOriginals({ kind: "move", assetIds: ids })}>
            <FolderInput aria-hidden="true" />
            {t("selection.action.move")}
          </ContextMenuItem>
          <ContextMenuItem
            variant="destructive"
            disabled={reason !== null}
            onSelect={() => askOriginals({ kind: "delete", assetIds: ids })}
          >
            <Trash2 aria-hidden="true" className="text-destructive" />
            {t("selection.action.delete")}
          </ContextMenuItem>
        </>
      )}
    </ContextMenuContent>
  )
}
