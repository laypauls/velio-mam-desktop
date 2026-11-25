import type { CardSize, GalleryCardDisplay, MediaType } from "@/lib/ipc-contract"

export type CardInfo = GalleryCardDisplay["info"]
export type CardInfoKey = keyof CardInfo
export type CardPresetName = "off" | "compact" | "detailed"
export type CardPresetOrCustom = CardPresetName | "custom"
export type CardDisplayPatch = Partial<Omit<GalleryCardDisplay, "info">> & { info?: Partial<CardInfo> }

/** 카드 폭이 이보다 좁으면 옵션과 무관하게 2줄(메타)·확장자 칩을 숨긴다 */
export const NARROW_CELL = 140
/** 카드 폭이 이보다 좁으면 상태 칩은 아이콘만 */
export const COMPACT_CHIP_CELL = 160

export const INFO_KEYS = ["typeIcon", "name", "ext", "resolution", "size", "date"] as const satisfies readonly CardInfoKey[]

export const PRESETS: Record<CardPresetName, CardInfo> = {
  off: { typeIcon: false, name: false, ext: false, resolution: false, size: false, date: false },
  compact: { typeIcon: true, name: true, ext: true, resolution: false, size: false, date: false },
  detailed: { typeIcon: true, name: true, ext: true, resolution: true, size: true, date: true },
}

export const DEFAULT_CARD_DISPLAY: GalleryCardDisplay = {
  info: { ...PRESETS.detailed },
  dateBy: "captured",
  check: "hover",
  favorite: true,
  rating: true,
  duration: true,
  quickActions: true,
  cardSize: "m",
}

/** 카드 크기 단계별 목표 폭(px) — 실제 폭은 `gridLayout` 이 줄을 꽉 채우도록 목표 근처에서 조정한다. 작은 순서 */
export const CARD_SIZES = { xs: 120, s: 150, m: 180, l: 240, xl: 300 } as const satisfies Record<CardSize, number>
export const CARD_SIZE_KEYS = Object.keys(CARD_SIZES) as CardSize[]

/**
 * 격자 열 수·카드 폭. 목표 폭에 가장 가까운 열 수를 고르고(round), 간격 `gap` 은 고정한 채 카드 폭을 소수 px 로
 * 늘이거나 줄여 줄을 꽉 채운다(오른쪽 잔여 0). 한 칸보다 좁으면 1열로 줄어든다.
 */
export function gridLayout(inner: number, target: number, gap: number): { cols: number; cell: number } {
  if (!(inner > 0)) return { cols: 1, cell: 0 }
  const cols = Math.max(1, Math.round((inner + gap) / (target + gap)))
  return { cols, cell: (inner - gap * (cols - 1)) / cols }
}

/** 정보 영역 6개 항목 조합이 미리 설정과 같으면 그 이름, 아니면 `custom` — 미리 설정은 저장하지 않고 항상 계산한다 */
export function presetOf(display: GalleryCardDisplay): CardPresetOrCustom {
  const names = Object.keys(PRESETS) as CardPresetName[]
  return names.find((n) => INFO_KEYS.every((k) => PRESETS[n][k] === display.info[k])) ?? "custom"
}

/** 정보 영역 1줄(유형 아이콘·파일명·확장자). 좁은 카드는 확장자 칩이 숨겨진다 */
export function hasNameLine(display: GalleryCardDisplay, cellWidth: number): boolean {
  const { typeIcon, name, ext } = display.info
  return typeIcon || name || (ext && cellWidth >= NARROW_CELL)
}

/** 정보 영역 2줄(해상도·크기·날짜). `mediaType` 이 음성이면 해상도는 기여하지 않는다 — 생략하면 해상도를 포함해 최대 줄 수를 낸다 */
export function hasMetaLine(display: GalleryCardDisplay, cellWidth: number, mediaType?: MediaType): boolean {
  if (cellWidth < NARROW_CELL) return false
  const { resolution, size, date } = display.info
  return (resolution && mediaType !== "audio") || size || date
}

/**
 * 정보 영역의 줄 수(0/1/2). 카드와 격자가 같은 함수를 쓴다 — 격자 행 높이는 `mediaType` 없이(유형 중 최대) 계산해
 * 같은 행의 카드 높이가 항상 같다.
 */
export function cardInfoLines(display: GalleryCardDisplay, cellWidth: number, mediaType?: MediaType): 0 | 1 | 2 {
  return ((hasNameLine(display, cellWidth) ? 1 : 0) + (hasMetaLine(display, cellWidth, mediaType) ? 1 : 0)) as 0 | 1 | 2
}

const INFO_PAD_Y = 14
/** 정보 영역 한 줄 높이(px) — 카드의 `h-[18px]` 와 같아야 한다 */
export const INFO_LINE_H = 18
const INFO_LINE_GAP = 2

/** 줄 수에 따른 정보 영역 높이(px). 0줄이면 영역 자체가 없다 */
export function infoHeight(lines: 0 | 1 | 2): number {
  return lines === 0 ? 0 : INFO_PAD_Y + INFO_LINE_H * lines + INFO_LINE_GAP * (lines - 1)
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v)
const bool = (v: unknown, d: boolean): boolean => (typeof v === "boolean" ? v : d)
const oneOf = <T extends string>(v: unknown, allowed: readonly T[], d: T): T => (allowed.includes(v as T) ? (v as T) : d)

/** 저장값(알 수 없는 형태 포함)을 안전한 값으로 — 빠진/틀린 필드는 필드 단위 기본값, 모르는 필드는 버린다 */
export function normalize(raw: unknown): GalleryCardDisplay {
  const d = DEFAULT_CARD_DISPLAY
  if (!isObj(raw)) return structuredClone(d)
  const info = isObj(raw.info) ? raw.info : {}
  return {
    info: {
      typeIcon: bool(info.typeIcon, d.info.typeIcon),
      name: bool(info.name, d.info.name),
      ext: bool(info.ext, d.info.ext),
      resolution: bool(info.resolution, d.info.resolution),
      size: bool(info.size, d.info.size),
      date: bool(info.date, d.info.date),
    },
    dateBy: oneOf(raw.dateBy, ["captured", "modified"], d.dateBy),
    check: oneOf(raw.check, ["hover", "always", "hidden"], d.check),
    favorite: bool(raw.favorite, d.favorite),
    rating: bool(raw.rating, d.rating),
    duration: bool(raw.duration, d.duration),
    quickActions: bool(raw.quickActions, d.quickActions),
    cardSize: oneOf(raw.cardSize, CARD_SIZE_KEYS, d.cardSize),
  }
}
