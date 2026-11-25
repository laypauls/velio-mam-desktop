/**
 * 구간별 개수만으로 만드는 행 모델 — 항목·행 배열을 만들지 않는다 (O(구간 수)).
 * 범위 [rangeStart, rangeEnd) 는 무한 스크롤이면 [0,total), 페이지 모드면 그 쪽의 조각.
 */

export interface SectionCount {
  key: string
  count: number
}

export interface SectionLayoutParams {
  cols: number
  /** false 면 구간·머리글 없이 범위 전체를 하나의 격자로 */
  grouped: boolean
  headerH: number
  /** 행 높이(카드 + 간격) */
  rowH: number
}

export interface VisibleSection {
  sectionIndex: number
  key: string
  /** 전역 첫 항목 위치 (범위로 잘린 뒤) */
  start: number
  count: number
  startY: number
  headerH: number
  rows: number
  height: number
  /** 범위 시작이 구간 중간 — 머리글에 "(이어짐)" 표시 */
  continued: boolean
}

export interface SectionModel {
  sections: VisibleSection[]
  totalHeight: number
  rangeStart: number
  rangeEnd: number
  cols: number
  rowH: number
}

export function buildSectionModel(
  input: readonly SectionCount[],
  rangeStart: number,
  rangeEnd: number,
  p: SectionLayoutParams,
): SectionModel {
  const cols = Math.max(1, Math.floor(p.cols))
  const sections: VisibleSection[] = []
  let y = 0
  const push = (sectionIndex: number, key: string, start: number, count: number, continued: boolean) => {
    const headerH = p.grouped ? p.headerH : 0
    const rows = Math.ceil(count / cols)
    const height = headerH + rows * p.rowH
    sections.push({ sectionIndex, key, start, count, startY: y, headerH, rows, height, continued })
    y += height
  }
  if (rangeEnd > rangeStart) {
    if (!p.grouped) {
      push(0, "all", rangeStart, rangeEnd - rangeStart, false)
    } else {
      let offset = 0
      for (let i = 0; i < input.length && offset < rangeEnd; i++) {
        const secStart = offset
        offset += input[i].count
        const a = Math.max(secStart, rangeStart)
        const z = Math.min(offset, rangeEnd)
        if (z > a) push(i, input[i].key, a, z - a, a > secStart)
      }
    }
  }
  return { sections, totalHeight: y, rangeStart, rangeEnd, cols, rowH: p.rowH }
}

/** y 를 포함하는(또는 마지막으로 시작이 y 이하인) 구간 번호 — 없으면 -1 */
function sectionAtY(m: SectionModel, y: number): number {
  let lo = 0
  let hi = m.sections.length - 1
  let ans = -1
  while (lo <= hi) {
    const mid = (lo + hi) >> 1
    if (m.sections[mid].startY <= y) {
      ans = mid
      lo = mid + 1
    } else hi = mid - 1
  }
  return ans
}

function sectionOfItem(m: SectionModel, g: number): number {
  let lo = 0
  let hi = m.sections.length - 1
  while (lo <= hi) {
    const mid = (lo + hi) >> 1
    const s = m.sections[mid]
    if (g < s.start) hi = mid - 1
    else if (g >= s.start + s.count) lo = mid + 1
    else return mid
  }
  return -1
}

/** y 위치 행의 첫 항목(전역). 머리글 위면 그 구간 첫 항목. 범위 밖 y 는 처음/마지막 행으로 맞춘다. 빈 모델은 null */
export function itemAt(m: SectionModel, y: number): number | null {
  if (m.sections.length === 0) return null
  const s = m.sections[Math.max(0, sectionAtY(m, Math.max(0, y)))]
  const row = Math.min(s.rows - 1, Math.max(0, Math.floor((y - s.startY - s.headerH) / m.rowH)))
  return s.start + row * m.cols
}

/** 전역 항목 g 가 놓인 행의 위쪽 y (구간 머리글 아래). 범위 밖이면 null */
export function offsetOfItem(m: SectionModel, g: number): number | null {
  const i = sectionOfItem(m, g)
  if (i < 0) return null
  const s = m.sections[i]
  return s.startY + s.headerH + Math.floor((g - s.start) / m.cols) * m.rowH
}

/** 구간 머리글 위쪽 y (레일 클릭 이동용). 범위에 없는 구간은 null */
export function offsetOfSection(m: SectionModel, sectionIndex: number): number | null {
  return m.sections.find((s) => s.sectionIndex === sectionIndex)?.startY ?? null
}

/** 뷰포트 [y0, y1) 와 겹치는 구간들 (머리글 포함) — 화면에 그릴 구간만 고를 때 */
export function sectionsInRange(m: SectionModel, y0: number, y1: number): VisibleSection[] {
  if (m.sections.length === 0 || y1 <= y0 || y1 <= 0 || y0 >= m.totalHeight) return []
  const out: VisibleSection[] = []
  for (let i = Math.max(0, sectionAtY(m, Math.max(0, y0))); i < m.sections.length && m.sections[i].startY < y1; i++) out.push(m.sections[i])
  return out
}

/**
 * 항목 g 가 놓인 행의 세로 범위 [top, bottom) — 구간의 첫 행이면 top 이 머리글 위쪽이다
 * (이동할 때 머리글까지 보이게). 범위 밖이면 null.
 */
export function itemSpan(m: SectionModel, g: number): { top: number; bottom: number } | null {
  const i = sectionOfItem(m, g)
  if (i < 0) return null
  const s = m.sections[i]
  const row = Math.floor((g - s.start) / m.cols)
  const rowTop = s.startY + s.headerH + row * m.rowH
  return { top: row === 0 ? s.startY : rowTop, bottom: rowTop + m.rowH }
}

/** y 가 속한 구간 (머리글 포함). 빈 모델은 null */
export function sectionAt(m: SectionModel, y: number): VisibleSection | null {
  if (m.sections.length === 0) return null
  return m.sections[Math.max(0, sectionAtY(m, Math.max(0, y)))]
}

/**
 * 뷰포트 [y0, y1) 에 (일부라도) 보이는 항목의 전역 구간 [first, last] (포함). 보이는 항목이 없으면 null.
 * 행 단위로 확장한다 (행의 모든 열 포함, 구간 끝 부분 행은 남은 항목까지).
 */
export function visibleItemRange(m: SectionModel, y0: number, y1: number): [number, number] | null {
  if (m.sections.length === 0 || y1 <= y0 || y1 <= 0 || y0 >= m.totalHeight) return null
  const top = Math.max(0, y0)
  const bottom = Math.min(m.totalHeight, y1) - 1e-6
  const sa = m.sections[Math.max(0, sectionAtY(m, top))]
  const sb = m.sections[Math.max(0, sectionAtY(m, bottom))]
  const first = sa.start + Math.min(sa.rows - 1, Math.max(0, Math.floor((top - sa.startY - sa.headerH) / m.rowH))) * m.cols
  const rowB = Math.floor((bottom - sb.startY - sb.headerH) / m.rowH)
  // 아래 끝이 머리글 안이면 그 구간 항목은 아직 안 보인다 → 이전 항목까지
  const last = rowB < 0 ? sb.start - 1 : Math.min(sb.start + sb.count - 1, sb.start + (Math.min(sb.rows - 1, rowB) + 1) * m.cols - 1)
  return last >= first ? [first, last] : null
}
