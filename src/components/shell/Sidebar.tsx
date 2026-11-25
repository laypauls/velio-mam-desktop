import { FileAudio, FileImage, FileVideo, Heart, Images, Library, Plus, Tags } from "lucide-react"
import type { LucideIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import { useAddFolder } from "@/hooks/useAddFolder"
import { useJobSummary } from "@/hooks/useJobSummary"
import { useSettings } from "@/hooks/useSettings"
import { useFolders } from "@/hooks/useLibrary"
import { useActionsStore, useSearchStore, useViewStore } from "@/stores"
import type { SidebarTab } from "@/stores/view"
import { FolderTree } from "./FolderTree"
import { AddFolderDialog } from "@/components/folders/AddFolderDialog"
import { RatingList } from "./RatingGroup"
import { SidebarCollections } from "./SidebarCollections"
import { Group, NavItem, sameView } from "./SidebarParts"

const LIBRARY: { kind: "all" | "image" | "video" | "audio" | "favorites"; icon: LucideIcon }[] = [
  { kind: "all", icon: Library },
  { kind: "image", icon: FileImage },
  { kind: "video", icon: FileVideo },
  { kind: "audio", icon: FileAudio },
  { kind: "favorites", icon: Heart },
]

export function Sidebar() {
  const { t } = useTranslation()
  const view = useSearchStore((s) => s.view)
  const setView = useSearchStore((s) => s.setView)
  const jobs = useJobSummary()
  const folders = useFolders()
  const addFolder = useAddFolder()
  const tab = useViewStore((s) => s.sidebarTab)
  const setTab = useViewStore((s) => s.setSidebarTab)
  const setTagManagerOpen = useActionsStore((s) => s.setTagManagerOpen)
  // 설정이 도착하기 전에는 그리지 않는다(개수 질의도 보류). 꺼진 상태의 별점 보기 복귀는 useViewGuards
  const ratingMenu = useSettings().data?.sidebarRatingMenu
  // 숨긴 관리 폴더는 사이드바에 보이지 않는다 (보기 복귀는 useViewGuards)
  const visibleFolders = folders.data?.filter((f) => !f.hidden)

  return (
    <nav aria-label={t("sidebar.label")} className="flex w-64 shrink-0 flex-col gap-3 overflow-y-auto border-r border-border p-2">
      <Tabs value={tab} onValueChange={(v) => setTab(v as SidebarTab)}>
        <TabsList aria-label={t("sidebar.tab.label")} className="w-full">
          <TabsTrigger value="library">
            <Library aria-hidden="true" />
            {t("sidebar.tab.library")}
          </TabsTrigger>
          <TabsTrigger value="collections">
            <Images aria-hidden="true" />
            {t("sidebar.tab.collections")}
          </TabsTrigger>
        </TabsList>
        <TabsContent value="library" className="flex flex-col gap-4 pt-1">
          <Group title={t("sidebar.library")}>
            <ul>
              {LIBRARY.map(({ kind, icon }) => (
                <NavItem
                  key={kind}
                  icon={icon}
                  label={t(`sidebar.view.${kind}`)}
                  count={kind === "all" ? jobs.data?.totalAssets : undefined}
                  active={sameView(view, { kind })}
                  onClick={() => setView({ kind })}
                />
              ))}
            </ul>
          </Group>

          {ratingMenu === true && (
            <Group title={t("sidebar.rating")}>
              <RatingList />
            </Group>
          )}

          <Group title={t("sidebar.tags")}>
            <Button size="sm" variant="ghost" className="justify-start" onClick={() => setTagManagerOpen(true)}>
              <Tags aria-hidden="true" />
              {t("sidebar.manageTags")}
            </Button>
          </Group>

          <Group
            title={t("sidebar.folders")}
            action={
              <Button size="icon-xs" variant="ghost" aria-label={t("sidebar.addFolder")} disabled={addFolder.isPending} onClick={() => addFolder.start()}>
                <Plus aria-hidden="true" />
              </Button>
            }
          >
            {visibleFolders && visibleFolders.length > 0 ? (
              <FolderTree folders={visibleFolders} allFolders={folders.data} />
            ) : (
              <p className="px-2 text-xs break-keep text-muted-foreground">{t(folders.data && folders.data.length > 0 ? "sidebar.foldersAllHidden" : "sidebar.foldersEmpty")}</p>
            )}
          </Group>
        </TabsContent>
        <TabsContent value="collections" className="pt-1">
          <SidebarCollections />
        </TabsContent>
      </Tabs>
      <AddFolderDialog flow={addFolder} />
    </nav>
  )
}
