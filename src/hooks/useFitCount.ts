import { useState } from "react"
import { useElementWidth } from "@/components/gallery/useElementWidth"

/**
 * 고정 폭 아이콘 버튼 `total` 개가 `width` 안에 몇 개 들어가는지 계산한다.
 * 전부 들어가면 `total`(⋯ 불필요), 아니면 ⋯ 버튼 한 칸을 남기고 들어가는 개수를 돌려준다.
 * 폭 0 이하는 아직 측정 전이므로 전부 보이는 것으로 본다.
 */
export function calcVisibleCount(width: number, total: number, itemPx: number, gapPx: number): number {
  if (total <= 0 || width <= 0) return Math.max(total, 0)
  const slots = Math.floor((width + gapPx) / (itemPx + gapPx))
  if (slots >= total) return total
  return Math.max(slots - 1, 0)
}

/** 요소 폭에 맞춰 인라인으로 보일 항목 수를 돌려준다: `[callbackRef, count]`. */
export function useFitCount(total: number, itemPx: number, gapPx: number): [(el: HTMLElement | null) => void, number] {
  const [el, setEl] = useState<HTMLElement | null>(null)
  const width = useElementWidth(el)
  return [setEl, calcVisibleCount(width, total, itemPx, gapPx)]
}
