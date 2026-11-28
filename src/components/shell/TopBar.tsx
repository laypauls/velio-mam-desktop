import { UpdateIndicator } from "@/components/update/UpdateIndicator"
import { FilterPopover } from "./FilterPopover"
import { SortControl, ViewControls } from "./ViewControls"
import { TopBarMoreMenu } from "./TopBarMoreMenu"
import { TopBarSearch } from "./TopBarSearch"
import { WindowControls } from "./WindowControls"

/**
 * 상단 바 = 사용자 정의 제목 표시줄 — 레일 오른쪽에 고정(보기 영역이 열려도 위치 불변), 빈 곳을 끌면 창이 움직이고 더블클릭하면 최대화/복원된다.
 * 드래그 영역(`data-tauri-drag-region`)은 **클릭된 요소 자신에만** 적용되므로 헤더·안쪽 래퍼·늘어나는 빈 칸에만 붙이고, 검색·버튼 같은 조작 요소에는 붙이지 않는다.
 * 폭에 따라 단계적으로 접힌다. 판정은 창 폭이 아니라 이 헤더(`@container`)의 폭이다(창 버튼 138px 포함).
 * - < 706px: 정렬 Select → 아이콘 메뉴(`SortControl`), 필터 라벨 숨김(`FilterPopover`)
 * - < 486px: 검색 박스 → 돋보기 아이콘 + 펼침 입력(`TopBarSearch`), 간격·여백 축소, 업데이트 표시 라벨 숨김
 * - < 426px: 정렬·정렬 방향·보기·카드 표시·업데이트를 "더보기"(⋯, `TopBarMoreMenu`) 하나로 모은다. 남는 것은 검색·필터·⋯·창 버튼
 * 이전 기준(640/420/360)에서 ☰·⚙(72px)가 빠지고 창 버튼(138px)이 더해져 사용 가능한 폭이 66px 줄어든 만큼 올렸다.
 * 컨테이너는 자기 자신을 쿼리할 수 없어 안쪽 래퍼가 간격을 줄인다. 높이는 레일 로고 칸·설정 헤더와 같은 `--chrome-h`.
 */
export function TopBar() {
  return (
    <header data-testid="topbar" data-tauri-drag-region className="@container relative h-(--chrome-h) shrink-0 border-b border-border">
      <div data-tauri-drag-region className="flex h-full items-center gap-2 pl-3 @max-[486px]:gap-1 @max-[486px]:pl-2">
        <TopBarSearch />
        <FilterPopover />
        <div data-tauri-drag-region className="h-full min-w-4 flex-1" />
        <div className="flex shrink-0 items-center gap-2 @max-[426px]:hidden @max-[486px]:gap-1">
          <SortControl />
          <ViewControls />
          <UpdateIndicator />
        </div>
        <TopBarMoreMenu />
        <WindowControls />
      </div>
    </header>
  )
}
