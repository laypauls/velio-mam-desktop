import { useMemo } from "react"
import { buildSectionModel, itemAt, itemSpan, visibleItemRange, type SectionModel } from "@/hooks/galleryWindow/sections"
import type { GalleryCardDisplay, SearchIndex, SortField } from "@/lib/ipc-contract"
import type { GalleryLayoutMode } from "@/stores/view"
import { CARD_SIZES, cardInfoLines, gridLayout, infoHeight } from "./cardDisplay"
import { groupField } from "./grouping"

export const GAP = 8
export const PAD = 12
export const HEADER_H = 36
export const LIST_ROW_H = 36
export const LIST_HEAD_H = 32

/**
 * 화면 배치 — 섹션 모델과 행 치수. 좌표계: "콘텐츠 y" = 모델 y + `padTop`. 스크롤 오프셋은 콘텐츠 y 기준이다.
 * (목록 보기의 열 머리글은 `padTop` 만큼의 고정 영역, 월 머리글 없는 격자는 위쪽 여백 `PAD`)
 */
export interface GalleryGeometry {
  model: SectionModel
  kind: GalleryLayoutMode
  grouped: boolean
  cols: number
  /** 카드 폭(격자) — 목록은 0 */
  cell: number
  cellH: number
  rowH: number
  padTop: number
  /** 스크롤 가능한 전체 높이 (실제 DOM 높이가 아니다 — 33.5M px 를 넘을 수 있다) */
  contentH: number
  viewportH: number
}

interface Inputs {
  viewMode: GalleryLayoutMode
  width: number
  viewportH: number
  display: GalleryCardDisplay
  index: SearchIndex | undefined
  range: [number, number]
  shownSortField: SortField
}

export function useGalleryGeometry({ viewMode, width, viewportH, display, index, range, shownSortField }: Inputs): GalleryGeometry {
  const grouped = viewMode === "grid" && index?.kind === "month" && groupField(shownSortField) !== null
  const { cols, cell } = viewMode === "grid" ? gridLayout(Math.max(0, width - PAD * 2), CARD_SIZES[display.cardSize], GAP) : { cols: 1, cell: 0 }
  // 카드 높이 = 셀 폭 + 정보 영역. 줄 수는 카드와 같은 순수 함수(유형 무관 최대값)로 계산해 행 안의 카드 높이가 같다
  const cellH = viewMode === "grid" ? cell + infoHeight(cardInfoLines(display, cell)) : LIST_ROW_H
  const rowH = viewMode === "grid" ? cellH + GAP : LIST_ROW_H
  const sections = index?.sections
  const model = useMemo(
    () => buildSectionModel(sections ?? [], range[0], range[1], { cols, grouped, headerH: HEADER_H, rowH }),
    [sections, range, cols, grouped, rowH],
  )
  const padTop = viewMode === "list" ? LIST_HEAD_H : grouped ? 0 : PAD
  const contentH = model.totalHeight + padTop + (viewMode === "grid" ? PAD : 0)
  return useMemo(
    () => ({ model, kind: viewMode, grouped, cols, cell, cellH, rowH, padTop, contentH, viewportH }),
    [model, viewMode, grouped, cols, cell, cellH, rowH, padTop, contentH, viewportH],
  )
}

/** 화면 맨 위(머리글 아래)에 걸친 행의 첫 항목 — 없으면 null */
export function firstVisibleItem(g: GalleryGeometry, offset: number): number | null {
  return itemAt(g.model, offset)
}

/** 화면에 (일부라도) 보이는 항목 구간 [first, last] — 없으면 null */
export function visibleItems(g: GalleryGeometry, offset: number): [number, number] | null {
  return visibleItemRange(g.model, offset - g.padTop, offset - g.padTop + g.viewportH)
}

/** 항목이 화면에 들어오는 최소 이동 후 오프셋("auto" 정렬) — 이미 보이면 그대로. 범위 밖이면 null */
export function offsetToReveal(g: GalleryGeometry, item: number, offset: number): number | null {
  const span = itemSpan(g.model, item)
  if (!span) return null
  // 목록의 열 머리글은 스크롤하지 않는 고정 영역이라 그 아래만 "보이는 곳"이다
  const sticky = g.kind === "list" ? g.padTop : 0
  const top = span.top + g.padTop
  const bottom = span.bottom + g.padTop
  if (top < offset + sticky || bottom - top > g.viewportH - sticky) return span.top
  if (bottom > offset + g.viewportH) return bottom - g.viewportH
  return offset
}

/** 항목이 든 행(구간 첫 행이면 월 머리글)이 화면 맨 위에 오도록 하는 오프셋. 범위 밖이면 null */
export function offsetOfItemTop(g: GalleryGeometry, item: number): number | null {
  return itemSpan(g.model, item)?.top ?? null
}
