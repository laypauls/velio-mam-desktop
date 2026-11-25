import { Menu, Settings } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { UpdateIndicator } from "@/components/update/UpdateIndicator"
import { useViewStore } from "@/stores"
import { FilterPopover } from "./FilterPopover"
import { SortControl, ViewControls } from "./ViewControls"
import { TopBarMoreMenu } from "./TopBarMoreMenu"
import { TopBarSearch } from "./TopBarSearch"

/**
 * 상단 바 — 폭에 따라 단계적으로 접힌다. 판정은 창 폭이 아니라 이 헤더(`@container`)의 폭이라 보기 영역 배치·사이드바와 무관하다.
 * - < 640px: 정렬 Select → 아이콘 메뉴(`SortControl`), 필터 라벨 숨김(`FilterPopover`)
 * - < 420px: 검색 박스 → 돋보기 아이콘 + 펼침 입력(`TopBarSearch`), 간격·여백 축소, 업데이트 표시 라벨 숨김
 * - < 360px: 정렬·정렬 방향·보기·카드 표시·업데이트·설정을 "더보기"(⋯, `TopBarMoreMenu`) 하나로 모은다. 남는 것은 메뉴·검색·필터·⋯
 * 컨테이너는 자기 자신을 쿼리할 수 없어 안쪽 래퍼가 간격을 줄인다. 이 최소 폭은 `viewerFit.TOPBAR_MIN_WIDTH` 와 맞춘다.
 */
export function TopBar({ onOpenSettings }: { onOpenSettings?: () => void }) {
  const { t } = useTranslation()
  const sidebarOpen = useViewStore((s) => s.sidebarOpen)
  const toggleSidebar = useViewStore((s) => s.toggleSidebar)
  return (
    <header className="@container relative border-b border-border">
      <div className="flex items-center gap-2 px-3 py-2 @max-[420px]:gap-1 @max-[420px]:px-2">
        <Button variant="ghost" size="icon" aria-label={t("shell.toggleSidebar")} aria-expanded={sidebarOpen} onClick={toggleSidebar}>
          <Menu aria-hidden="true" />
        </Button>
        <TopBarSearch />
        <FilterPopover />
        <div className="ml-auto flex shrink-0 items-center gap-2 @max-[360px]:hidden @max-[420px]:gap-1">
          <SortControl />
          <ViewControls />
          <UpdateIndicator />
          <Button variant="ghost" size="icon" aria-label={t("shell.settings")} title={t("shell.settings")} onClick={onOpenSettings}>
            <Settings aria-hidden="true" />
          </Button>
        </div>
        <TopBarMoreMenu onOpenSettings={onOpenSettings} />
      </div>
    </header>
  )
}
