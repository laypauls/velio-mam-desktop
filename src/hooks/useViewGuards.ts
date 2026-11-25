import { useEffect } from "react"
import { useSearchStore } from "@/stores"
import { useFolders } from "./useLibrary"
import { useSettings } from "./useSettings"

/**
 * 더 이상 보여 주면 안 되는 보기를 "전체" 로 돌린다. 사이드바가 닫혀 있어도 동작해야 하므로 항상 마운트되는 AppShell 에서 쓴다.
 * - 보던 관리 폴더가 숨김이 됐다 (토글 성공 후 폴더 목록이 다시 로드될 때)
 * - 별점 메뉴가 꺼진 상태에서 별점 보기가 선택돼 있다
 */
export function useViewGuards(): void {
  const view = useSearchStore((s) => s.view)
  const setView = useSearchStore((s) => s.setView)
  const folders = useFolders().data
  const ratingMenu = useSettings().data?.sidebarRatingMenu
  const ratingViewHidden = ratingMenu === false && view.kind === "rating"
  const folderViewHidden = view.kind === "folder" && (folders?.some((f) => f.folderId === view.folderId && f.hidden) ?? false)
  const reset = ratingViewHidden || folderViewHidden
  useEffect(() => {
    if (reset) setView({ kind: "all" })
  }, [reset, setView])
}
