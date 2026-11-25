/** 유사도 띠의 칸 수 — 한 칸이 슬라이더 한 단계(5%)이고, 칸 i 는 값 5·i % 를 나타낸다(0, 5, …, 95) */
export const BAND_CELLS = 20
const STEP = 5

/** 띠의 구역: 무관한 이미지가 섞이는 낮은 구간 / 중간 / 권장 구간 */
export type BandZone = "weak" | "mid" | "recommended"

/** 이 값 이하(0~60%)는 무관한 이미지가 섞이고, 이 값 이상(70%+)은 권장 — 사이는 중간 */
export const WEAK_UP_TO = 60
export const RECOMMENDED_FROM = 70

export function similarityZone(percent: number): BandZone {
  if (percent <= WEAK_UP_TO) return "weak"
  return percent < RECOMMENDED_FROM ? "mid" : "recommended"
}

/** 현재 값이 놓이는 띠의 칸(0~19). 0% = 첫 칸, 100% 이상 = 마지막 칸, 숫자가 아니면 첫 칸 */
export function markerCell(percent: number): number {
  if (!Number.isFinite(percent)) return 0
  return Math.min(BAND_CELLS - 1, Math.max(0, Math.round(percent / STEP)))
}

/** 칸(0~19)이 나타내는 구역 */
export function cellZone(cell: number): BandZone {
  return similarityZone(cell * STEP)
}
