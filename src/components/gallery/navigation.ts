/** 갤러리 키보드 이동의 순수 계산 — 가상화·DOM 과 무관하게 "이동 가능한 범위 안의 0-기준 위치"만 다룬다. */

/** 격자/목록이 정하는 이동 규칙 */
export interface GalleryNav {
  /** 한 행의 항목 수 (목록은 1) */
  cols: number
  /** PageUp/PageDown 이동량(항목 수) */
  pageSize: number
  /** 좌/우 화살표 허용 여부 (격자만) */
  horizontal: boolean
}

export const NAV_KEYS = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"] as const

export function isNavKey(key: string): boolean {
  return (NAV_KEYS as readonly string[]).includes(key)
}

/** 키에 해당하는 다음 인덱스. 이동 대상이 아니면 null. `index` -1 = 활성 항목 없음(첫 이동은 처음/끝 항목으로). */
export function nextIndex(key: string, index: number, count: number, nav: GalleryNav): number | null {
  if (count <= 0) return null
  const last = count - 1
  const { cols, pageSize, horizontal } = nav
  if (key === "Home") return 0
  if (key === "End") return last
  if (index < 0) return isNavKey(key) && (horizontal || (key !== "ArrowLeft" && key !== "ArrowRight")) ? 0 : null
  switch (key) {
    case "ArrowRight":
      return horizontal ? Math.min(last, index + 1) : null
    case "ArrowLeft":
      return horizontal ? Math.max(0, index - 1) : null
    case "ArrowDown":
      if (index + cols <= last) return index + cols
      // 마지막 행이 덜 찼으면 마지막 항목으로, 이미 마지막 행이면 제자리
      return Math.floor(index / cols) < Math.floor(last / cols) ? last : index
    case "ArrowUp":
      return index - cols >= 0 ? index - cols : index
    case "PageDown":
      return Math.min(last, index + Math.max(1, pageSize))
    case "PageUp":
      return Math.max(0, index - Math.max(1, pageSize))
    default:
      return null
  }
}
