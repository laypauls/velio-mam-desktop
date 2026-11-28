import { VIEWER_MIN_WIDTH } from "@/stores/view"

/** 갤러리가 최소로 남겨야 하는 폭(px) */
export const GALLERY_MIN_WIDTH = 288
/** 섹션 패널 폭(px) — `w-60`. 보기 영역이 열려 폭이 모자라면 이 패널을 먼저 임시로 접는다 */
export const SECTION_PANEL_WIDTH = 240
/** 보기 영역 옆 분할선 폭(px) — `w-1.5` */
const RESIZE_HANDLE_WIDTH = 6
/** 상세 패널 폭(px) — 열림 `w-80` / 접힘(펼치기 버튼만 있는 좁은 띠) */
export const DETAIL_OPEN_WIDTH = 320
export const DETAIL_CLOSED_WIDTH = 41

/**
 * 보기 영역이 가질 수 있는 최대 폭 — 보기 영역과 형제 갤러리(`main`)가 함께 쓰는 폭(`total`)에서 갤러리 최소 폭을 뺀 값.
 * 보기 영역은 왼쪽·오른쪽 배치 모두 TopBar 아래 본문 행 안에서 갤러리와 나란히 놓이므로 계산이 같다(TopBar 는 영향이 없다).
 * 레일·섹션 패널·상세 패널은 `total` 에 들어 있지 않아 열림/접힘이 자동으로 반영된다.
 * 보기 영역 최소 폭(VIEWER_MIN_WIDTH)이 항상 우선한다 — 창이 더 좁으면 갤러리가 최소 폭 아래로 양보한다.
 * `total` 이 0 이하(레이아웃 없음)면 제한 없음(Infinity).
 */
export function viewerMaxWidth(total: number): number {
  if (!(total > 0)) return Number.POSITIVE_INFINITY
  return Math.max(VIEWER_MIN_WIDTH, total - GALLERY_MIN_WIDTH)
}

/** 레이아웃을 읽어 최대 폭을 계산한다 — 보기 영역·갤러리가 없으면 제한 없음. 보기 영역 자신의 폭 + 갤러리 폭은 분배와 무관한 합이라 되먹임이 없다. */
export function measureViewerMaxWidth(pane: HTMLElement | null): number {
  const main = pane?.parentElement?.querySelector(":scope > main")
  if (!pane || !main) return Number.POSITIVE_INFINITY
  return viewerMaxWidth(pane.getBoundingClientRect().width + main.getBoundingClientRect().width)
}

/** 저장된 폭을 [VIEWER_MIN_WIDTH, max] 로 맞춘 화면 표시 폭 — 저장 값은 바꾸지 않는다. */
export function fitViewerWidth(width: number, max: number): number {
  return Math.min(Math.max(width, VIEWER_MIN_WIDTH), Math.max(max, VIEWER_MIN_WIDTH))
}

/**
 * 본문 행(`rowWidth`)에 섹션 패널 + 보기 영역(최소 폭) + 분할선 + 갤러리(최소 폭) + 상세 패널이 모두 들어가는가.
 * 들어가지 않으면 섹션 패널을 임시로 접는다. 행 폭이 0 이하(레이아웃 없음)면 들어간다고 본다.
 */
export function panelFitsWithViewer(rowWidth: number, detailOpen: boolean): boolean {
  if (!(rowWidth > 0)) return true
  const detail = detailOpen ? DETAIL_OPEN_WIDTH : DETAIL_CLOSED_WIDTH
  return rowWidth >= SECTION_PANEL_WIDTH + VIEWER_MIN_WIDTH + RESIZE_HANDLE_WIDTH + GALLERY_MIN_WIDTH + detail
}
