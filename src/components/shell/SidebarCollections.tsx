import { Images, Plus } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useCollections } from "@/hooks/useLibrary"
import { useActionsStore, usePlaybackStore, useSearchStore } from "@/stores"
import { CollectionMenu } from "./CollectionMenu"
import { Group, NavItem } from "./SidebarParts"

/** 사이드바 "컬렉션" 탭 본문 — 컬렉션 목록(재생 중이면 ▷)과 추가 버튼. */
export function SidebarCollections() {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const setView = useSearchStore((s) => s.setView)
  const collections = useCollections()
  const openForm = useActionsStore((s) => s.openCollectionForm)
  const playingId = usePlaybackStore((s) => (s.status === "idle" ? null : s.collectionId))
  return (
    <Group
      title={t("sidebar.collections")}
      action={
        <Button size="icon-xs" variant="ghost" aria-label={t("sidebar.addCollection")} onClick={() => openForm({ mode: "create", assetIds: [] })}>
          <Plus aria-hidden="true" />
        </Button>
      }
    >
      {collections.data && collections.data.length > 0 ? (
        <ul>
          {collections.data.map((c) => (
            <NavItem
              key={c.collectionId}
              icon={Images}
              label={c.name}
              count={c.assetCount}
              active={view.kind === "collection" && view.collectionId === c.collectionId}
              playing={playingId === c.collectionId}
              onClick={() => setView({ kind: "collection", collectionId: c.collectionId })}
              menu={<CollectionMenu collection={c} />}
            />
          ))}
        </ul>
      ) : (
        <p className="px-2 text-xs break-keep text-muted-foreground">{t("sidebar.collectionsEmpty")}</p>
      )}
    </Group>
  )
}
