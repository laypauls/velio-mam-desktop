import { useEffect } from "react"
import { useSearchStore, useViewStore } from "@/stores"
import { useFolders } from "./useLibrary"
import { useSettings } from "./useSettings"

/**
 * 더 이상 보여 주면 안 되는 보기를 "전체" 로 돌린다. 섹션 패널이 접혀 있어도 동작해야 하므로 항상 마운트되는 AppShell 에서 쓴다 (레일·패널 구조에 의존하지 않는다).
 * - 보던 관리 폴더가 숨김이 됐다 (토글 성공 후 폴더 목록이 다시 로드될 때)
 * - 별점 메뉴가 꺼진 상태에서 별점 보기가 선택돼 있다
 * 별점 메뉴가 꺼진 상태에서 레일의 마지막 섹션이 "rating" 이면 "library" 로 보정한다(저장값도 함께 바뀐다).
 */
export function useViewGuards(): void {
  const view = useSearchStore((s) => s.view)
  const setView = useSearchStore((s) => s.setView)
  const folders = useFolders().data
  const ratingMenu = useSettings().data?.sidebarRatingMenu
  const ratingViewHidden = ratingMenu === false && view.kind === "rating"
  const folderViewHidden = view.kind === "folder" && (folders?.some((f) => f.folderId === view.folderId && f.hidden) ?? false)
  const section = useViewStore((s) => s.sidebarTab)
  const setSection = useViewStore((s) => s.setSidebarTab)
  const ratingSectionHidden = ratingMenu === false && section === "rating"
  const reset = ratingViewHidden || folderViewHidden
  useEffect(() => {
    if (reset) setView({ kind: "all" })
  }, [reset, setView])
  useEffect(() => {
    if (ratingSectionHidden) setSection("library")
  }, [ratingSectionHidden, setSection])
}
