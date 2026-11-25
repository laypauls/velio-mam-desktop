import type { AssetSummary } from "@/lib/ipc-contract"
import { FILMSTRIP_MIN_HEIGHT } from "@/stores/view"

/** 타일 사이 간격(px) */
export const TILE_GAP = 6
/** 스트립 위·아래 여백(px) — 타일 높이 = 스트립 높이 − 2 × 여백 */
export const STRIP_PAD_Y = 6
export const MIN_ASPECT = 0.75
export const MAX_ASPECT = 1.78
/** 보기 무대(stage)가 보장받는 최소 높이 */
export const STAGE_MIN_HEIGHT = 240
/** 툴바·높이 조절 손잡이 높이 — 창이 낮을 때 스트립이 줄어들 여유 계산에 쓴다 */
export const TOOLBAR_HEIGHT = 40
export const GRIP_HEIGHT = 8
/** 창 밖으로 미리 그려 두는 폭(px) */
const OVERSCAN_PX = 120

type Dim = Pick<AssetSummary, "mediaType" | "width" | "height">

/** 타일 가로세로비 — 원본 비율을 0.75~1.78 로 제한한다. 크기를 모르는 항목(음성·미지정)은 정사각 */
export function tileAspect(item: Dim | undefined): number {
  if (!item || item.mediaType === "audio" || !item.width || !item.height || item.width <= 0 || item.height <= 0) return 1
  return Math.min(MAX_ASPECT, Math.max(MIN_ASPECT, item.width / item.height))
}

export function tileWidth(tileH: number, item: Dim | undefined): number {
  return Math.max(1, Math.round(tileH * tileAspect(item)))
}

/** 저장된 높이를 현재 컨테이너에 맞춘 표시 높이 — 무대 최소 높이(240)를 먼저 지키고 스트립을 72 까지 줄인다. 컨테이너 높이를 모르면(0) 저장값 */
export function effectiveStripHeight(stored: number, containerH: number): number {
  if (containerH <= 0) return stored
  const room = containerH - TOOLBAR_HEIGHT - GRIP_HEIGHT - STAGE_MIN_HEIGHT
  return Math.max(FILMSTRIP_MIN_HEIGHT, Math.min(stored, room))
}

export interface StripWindow {
  /** 그릴 전역 위치 [first, last] (포함) */
  first: number
  last: number
  /** `first` 타일 왼쪽 가장자리의 x (뷰포트 기준) */
  offset: number
  /** 뷰포트에 (일부라도) 보이는 타일 수 */
  visible: number
}

interface WindowArgs {
  /** 가운데에 둘 전역 위치 — `range` 안이어야 한다 */
  center: number
  range: readonly [number, number]
  viewportW: number
  /** 위치별 타일 폭 */
  widthOf: (pos: number) => number
}

/**
 * 가운데 타일을 뷰포트 중앙에 두고 양옆으로 폭을 채워 그릴 구간을 계산한다.
 * 전체 폭 누적이 필요 없어(가변 폭 + 미로딩 항목) 100만 건에서도 보이는 만큼만 계산한다.
 */
export function stripWindow({ center, range: [r0, r1], viewportW, widthOf }: WindowArgs): StripWindow | null {
  if (r1 <= r0 || viewportW <= 0) return null
  const c = Math.min(r1 - 1, Math.max(r0, center))
  const cw = widthOf(c)
  const centerLeft = (viewportW - cw) / 2
  let first = c
  let offset = centerLeft
  while (first > r0 && offset > -OVERSCAN_PX) {
    first -= 1
    offset -= widthOf(first) + TILE_GAP
  }
  let last = c
  let right = centerLeft + cw
  while (last < r1 - 1 && right < viewportW + OVERSCAN_PX) {
    last += 1
    right += TILE_GAP + widthOf(last)
  }
  let visible = 0
  let x = offset
  for (let p = first; p <= last; p++) {
    const w = widthOf(p)
    if (x < viewportW && x + w > 0) visible += 1
    x += w + TILE_GAP
  }
  return { first, last, offset, visible }
}
