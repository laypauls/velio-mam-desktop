import { VIEWER_MIN_WIDTH } from "@/stores/view"

/** 갤러리가 최소로 남겨야 하는 폭(px) */
export const GALLERY_MIN_WIDTH = 288
/**
 * 왼쪽 배치에서 오른쪽 열이 남겨야 하는 TopBar 최소 폭(px) — **완전히 접힌 상태**(TopBar 의 `@max-[360px]` "더보기") 기준:
 * 남는 컨트롤은 메뉴·검색·필터(아이콘 + 개수 배지)·⋯ — 36 + 36 + 약 76 + 36 + 간격 3×4 + 좌우 여백 2×8 ≈ 212 로 갤러리 최소(카드 1개 폭)보다 작다.
 * 그래서 갤러리 최소와 같게 둔다. 사이드바·상세가 열려 있으면 갤러리 최소 + 그 폭이 더 크다. TopBar 의 접힘 구성이 바뀌면 함께 조정한다.
 */
export const TOPBAR_MIN_WIDTH = GALLERY_MIN_WIDTH

export type ViewerSide = "left" | "right"

/**
 * 보기 영역이 가질 수 있는 최대 폭 — 보기 영역과 형제가 함께 쓰는 폭(`total`)에서 형제가 남겨야 할 최소 폭을 뺀 값.
 * 오른쪽 배치: 형제는 `main`(갤러리), 최소 = 갤러리 최소 폭.
 * 왼쪽 배치: 형제는 TopBar·사이드바·갤러리·상세가 든 오른쪽 열, 최소 = max(TopBar 최소 폭, 갤러리 최소 + 열려 있는 사이드바·상세 폭(`chrome`)).
 * 보기 영역 최소 폭(VIEWER_MIN_WIDTH)이 항상 우선한다 — 창이 더 좁으면 갤러리가 최소 폭 아래로 양보한다.
 * `total` 이 0 이하(레이아웃 없음)면 제한 없음(Infinity).
 */
export function viewerMaxWidth(side: ViewerSide, total: number, chrome: number): number {
  if (!(total > 0)) return Number.POSITIVE_INFINITY
  const minSibling = side === "left" ? Math.max(TOPBAR_MIN_WIDTH, GALLERY_MIN_WIDTH + chrome) : GALLERY_MIN_WIDTH
  return Math.max(VIEWER_MIN_WIDTH, total - minSibling)
}

/** 레이아웃을 읽어 최대 폭을 계산한다 — 형제 영역이 없으면 제한 없음. 보기 영역 자신의 폭 + 형제 폭은 분배와 무관한 합이라 되먹임이 없다. */
export function measureViewerMaxWidth(pane: HTMLElement | null, side: ViewerSide): number {
  const parent = pane?.parentElement
  if (!pane || !parent) return Number.POSITIVE_INFINITY
  const sibling = parent.querySelector(side === "left" ? ":scope > [data-viewer-sibling]" : ":scope > main")
  if (!sibling) return Number.POSITIVE_INFINITY
  let chrome = 0
  if (side === "left") {
    for (const el of sibling.querySelector("main")?.parentElement?.children ?? []) {
      if (el.tagName !== "MAIN") chrome += el.getBoundingClientRect().width
    }
  }
  return viewerMaxWidth(side, pane.getBoundingClientRect().width + sibling.getBoundingClientRect().width, chrome)
}

/** 저장된 폭을 [VIEWER_MIN_WIDTH, max] 로 맞춘 화면 표시 폭 — 저장 값은 바꾸지 않는다. */
export function fitViewerWidth(width: number, max: number): number {
  return Math.min(Math.max(width, VIEWER_MIN_WIDTH), Math.max(max, VIEWER_MIN_WIDTH))
}
