import { Plus } from "lucide-react"
import { useTranslation } from "react-i18next"
import { AddFolderDialog } from "@/components/folders/AddFolderDialog"
import { Button } from "@/components/ui/button"
import { useAddFolder } from "@/hooks/useAddFolder"
import { useFolders } from "@/hooks/useLibrary"
import { FolderTree } from "./FolderTree"
import { Group } from "./SidebarParts"

/** "관리 폴더" 섹션 패널 — 폴더 트리 + 폴더 추가. 숨긴 관리 폴더는 보이지 않는다(보기 복귀는 useViewGuards). */
export function SidebarFolders() {
  const { t } = useTranslation()
  const folders = useFolders()
  const addFolder = useAddFolder()
  const visibleFolders = folders.data?.filter((f) => !f.hidden)
  return (
    <>
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
      <AddFolderDialog flow={addFolder} />
    </>
  )
}
