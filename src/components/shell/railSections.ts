import { Folder, Image, Images, Star, Tag } from "lucide-react"
import type { LucideIcon } from "lucide-react"
import type { SidebarView } from "@/stores"
import type { SidebarTab } from "@/stores/view"

/** 레일 섹션 순서·아이콘 — 별점은 설정 `sidebarRatingMenu` 가 켜져 있을 때만 보인다 */
export const RAIL_SECTIONS: { id: SidebarTab; icon: LucideIcon }[] = [
  { id: "library", icon: Image },
  { id: "rating", icon: Star },
  { id: "folders", icon: Folder },
  { id: "tags", icon: Tag },
  { id: "collections", icon: Images },
]

/** 섹션 패널 DOM id — 레일 버튼의 `aria-controls` 가 가리킨다 */
export const SIDEBAR_PANEL_ID = "sidebar-panel"

/** 갤러리의 현재 보기가 속한 섹션 (태그 필터는 보기와 별개라 `tagsActive` 로 따로 판단한다) */
export function viewSection(view: SidebarView): SidebarTab {
  switch (view.kind) {
    case "rating":
      return "rating"
    case "folder":
      return "folders"
    case "collection":
      return "collections"
    default:
      return "library"
  }
}

/** 별점 메뉴가 꺼져 있으면(또는 설정 도착 전) "rating" 은 "library" 로 보정한다 */
export function effectiveSection(tab: SidebarTab, ratingMenu: boolean | undefined): SidebarTab {
  return tab === "rating" && ratingMenu !== true ? "library" : tab
}
