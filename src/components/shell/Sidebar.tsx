import { useTranslation } from "react-i18next"
import { useSettings } from "@/hooks/useSettings"
import { useViewStore } from "@/stores/view"
import { SidebarCollections } from "./SidebarCollections"
import { SidebarFolders } from "./SidebarFolders"
import { SidebarLibrary, SidebarRating } from "./SidebarLibrary"
import { SidebarTags } from "./SidebarTags"
import { effectiveSection, SIDEBAR_PANEL_ID } from "./railSections"

/**
 * 레일 오른쪽의 섹션 패널(w-60) — 현재 섹션 하나만 그린다(라이브러리 / 별점 / 관리 폴더 / 태그 / 컬렉션).
 * 표시 여부(저장된 열림 상태 + 폭 부족 시 임시 접힘)는 호출부(AppShell)가 정한다.
 * 별점 섹션은 설정 `sidebarRatingMenu` 가 켜져 있을 때만 의미가 있어, 꺼져 있으면 라이브러리로 보정한다(저장 보정은 useViewGuards).
 */
export function Sidebar() {
  const { t } = useTranslation()
  const tab = useViewStore((s) => s.sidebarTab)
  const section = effectiveSection(tab, useSettings().data?.sidebarRatingMenu)
  return (
    // 스크롤은 각 섹션(Group)의 본문이 담당 — 제목 헤더는 고정
    <nav id={SIDEBAR_PANEL_ID} aria-label={t("sidebar.label")} className="flex w-60 shrink-0 flex-col border-r border-border">
      {section === "library" && <SidebarLibrary />}
      {section === "rating" && <SidebarRating />}
      {section === "folders" && <SidebarFolders />}
      {section === "tags" && <SidebarTags />}
      {section === "collections" && <SidebarCollections />}
    </nav>
  )
}
