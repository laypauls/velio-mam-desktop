import type { TFunction } from "i18next"
import type { SearchIndex, SearchIndexKind, SearchIndexSection } from "@/lib/ipc-contract"
import { monthSectionLabel } from "./grouping"

/** 레일 라벨 최소 간격(px) — 월 종류는 연도 라벨이라 조금 넓게 */
const MIN_GAP: Record<SearchIndexKind, number> = { month: 16, initial: 14, none: 0 }
/** 월 점이 이보다 많으면 점을 그리지 않는다 */
const MAX_DOTS = 260
/** 트랙 위아래에서 라벨이 잘리지 않게 두는 여백 */
const EDGE = 6

export interface RailLabel {
  key: string
  y: number
  text: string
}
export interface RailTicks {
  labels: RailLabel[]
  dots: number[]
}

/** 이름 목차의 짧은 라벨 — `sym` → #, a–z → 대문자, 기타(앞뒤 둘 다) → "기타", 초성은 그대로 */
function initialShort(key: string, t: TFunction): string {
  if (key === "sym") return "#"
  if (key === "etc" || key === "etc_end") return t("gallery.rail.etc")
  return /^[a-z]$/.test(key) ? key.toUpperCase() : key
}

/** 레일에 찍는 짧은 라벨 — 월 종류는 연도(구간이 그 해 첫 구간일 때만 의미 있다), 날짜 없음은 번역 문구 */
export function railLabel(kind: SearchIndexKind, key: string, t: TFunction): string | null {
  if (kind === "initial") return initialShort(key, t)
  if (kind === "month") return /^\d{4}-\d{2}$/.test(key) ? key.slice(0, 4) : t("gallery.noDate")
  return null
}

/** 말풍선·접근성 문구용 전체 라벨 — 월 종류는 "2024년 3월" / "날짜 없음", 이름은 짧은 라벨 그대로 */
export function sectionText(kind: SearchIndexKind, s: SearchIndexSection, t: TFunction): string | null {
  if (kind === "initial") return initialShort(s.key, t)
  if (kind === "month") return monthSectionLabel(s.key) ?? t("gallery.noDate")
  return null
}

/** 월 구간의 연도 묶음 키 — 날짜 없음은 자기 혼자 한 묶음 */
const yearOf = (key: string) => (/^\d{4}-\d{2}$/.test(key) ? key.slice(0, 4) : key)

/** 이름 목차의 문자 계열 — 계열이 바뀌는 첫 구간은 레일이 좁아도 라벨을 남긴다 */
export function scriptGroup(key: string): string {
  if (/^[a-z]$/.test(key)) return "latin"
  if (/^[ㄱ-ㅎ]$/.test(key)) return "hangul"
  return key // sym · etc · etc_end
}

/** 이름 목차(균등 간격)의 `i` 번째 칸 위치(트랙 비율) — 위아래 EDGE 만큼 안쪽에서 `n` 칸을 같은 간격으로 */
export function slotFraction(n: number, i: number, trackH: number): number {
  if (trackH <= 0) return 0
  const usable = Math.max(0, trackH - 2 * EDGE)
  return (EDGE + (n > 1 ? (i / (n - 1)) * usable : usable / 2)) / trackH
}

/** 트랙 비율 `f` 에서 가장 가까운 칸 */
export function slotAt(n: number, f: number, trackH: number): number {
  if (n <= 1 || trackH <= 0) return 0
  const usable = Math.max(1, trackH - 2 * EDGE)
  return Math.min(n - 1, Math.max(0, Math.round(((f * trackH - EDGE) / usable) * (n - 1))))
}

/** `starts`(누적 시작 위치, 길이 n+1)에서 전역 항목 `g` 가 든 구간 */
export function sectionOfItem(starts: readonly number[], g: number): number {
  let lo = 0
  let hi = starts.length - 2
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (starts[mid] <= g) lo = mid
    else hi = mid - 1
  }
  return Math.max(0, lo)
}

/** 이름 목차 핸들 위치(트랙 비율) — 첫 보이는 항목이 든 구간 안 진행도를 이웃 칸 사이로 보간한다 */
export function handleFraction(starts: readonly number[], firstItem: number, trackH: number): number {
  const n = starts.length - 1
  if (n <= 0) return 0
  const c = sectionOfItem(starts, firstItem)
  const count = Math.max(1, starts[c + 1] - starts[c])
  const p = Math.min(1, Math.max(0, (firstItem - starts[c]) / count))
  const a = slotFraction(n, c, trackH)
  const b = c + 1 < n ? slotFraction(n, c + 1, trackH) : a
  return a + p * (b - a)
}

/** 이름 목차 라벨 — 모든 구간이 자기 칸을 갖는다. 좁으면 계열 첫 구간·처음·마지막은 꼭 남기고 나머지를 간격에 맞춰 솎는다 */
function initialTicks(sections: readonly SearchIndexSection[], trackH: number, t: TFunction): RailTicks {
  const n = sections.length
  const usable = Math.max(1, trackH - 2 * EDGE)
  const slotSp = n > 1 ? usable / (n - 1) : usable
  const minIdx = Math.max(1, Math.ceil(MIN_GAP.initial / slotSp))
  const keep = new Set<number>([0, n - 1])
  sections.forEach((s, i) => {
    if (i > 0 && scriptGroup(s.key) !== scriptGroup(sections[i - 1].key)) keep.add(i)
  })
  for (let i = 0; i < n; i++) {
    if (keep.has(i)) continue
    if ([...keep].every((m) => Math.abs(m - i) >= minIdx)) keep.add(i)
  }
  const labels = [...keep]
    .sort((a, b) => a - b)
    .map((i) => ({ key: sections[i].key, y: slotFraction(n, i, trackH) * trackH, text: initialShort(sections[i].key, t) }))
  return { labels, dots: [] }
}

/**
 * 구간 위치(0..1, `fracs[i]`)로 레일 라벨·점을 만든다. 라벨은 위에서부터 욕심내어 겹치지 않게만 남긴다.
 * 월: 연도의 첫 구간에 연도 라벨 + 나머지 구간에 점(비례 배치). 이름: 구간마다 균등 간격 칸(`initialTicks`, `fracs` 안 씀). 그 밖: 없음.
 */
export function buildTicks(index: SearchIndex | undefined, fracs: readonly number[], trackH: number, t: TFunction): RailTicks {
  const out: RailTicks = { labels: [], dots: [] }
  if (!index || trackH <= 0 || index.kind === "none") return out
  const { kind, sections } = index
  if (kind === "initial") return initialTicks(sections, trackH, t)
  const gap = MIN_GAP[kind]
  let lastY = -Infinity
  let prevYear: string | null = null
  const labeled = new Set<number>()
  sections.forEach((s, i) => {
    const wanted = yearOf(s.key) !== prevYear
    prevYear = yearOf(s.key)
    if (!wanted) return
    const y = Math.min(trackH - EDGE, Math.max(EDGE, fracs[i] * trackH))
    if (y - lastY < gap) return
    const text = railLabel(kind, s.key, t)
    if (text === null) return
    out.labels.push({ key: s.key, y, text })
    labeled.add(i)
    lastY = y
  })
  if (kind === "month" && sections.length < MAX_DOTS) {
    sections.forEach((_, i) => {
      if (!labeled.has(i)) out.dots.push(fracs[i] * trackH)
    })
  }
  return out
}

/** `fracs`(오름차순)에서 위치 `f` 이하인 구간 — 같은 위치(같은 행)에 여러 구간이 있으면 그 첫 구간. 첫 구간 앞이면 0 */
export function sectionAtFraction(fracs: readonly number[], f: number): number {
  let lo = 0
  let hi = fracs.length - 1
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (fracs[mid] <= f) lo = mid
    else hi = mid - 1
  }
  while (lo > 0 && fracs[lo - 1] === fracs[lo]) lo--
  return Math.max(0, lo)
}
