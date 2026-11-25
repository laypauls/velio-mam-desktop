/** 콘텐츠 보기 영역의 순수 계산 — 프레임 이동·이미지 확대/이동. */

export const DEFAULT_FPS = 30
export const PLAYBACK_RATES = [0.25, 0.5, 1, 1.5, 2] as const
export const SEEK_STEP_SEC = 5

/** 재생 위치를 `[0, duration]` 안으로 맞춘다. duration 을 모르면(NaN/무한) 아래쪽만 막는다. */
export function clampTime(time: number, duration: number): number {
  const upper = Number.isFinite(duration) && duration > 0 ? duration : Number.POSITIVE_INFINITY
  return Math.min(Math.max(time, 0), upper)
}

/** 프레임 이동 후 위치(초). fps 가 없거나 0 이하이면 30 으로 계산한다. */
export function stepFrame(current: number, fps: number | null | undefined, dir: 1 | -1, duration: number): number {
  const rate = fps !== null && fps !== undefined && fps > 0 ? fps : DEFAULT_FPS
  return clampTime(current + dir / rate, duration)
}

export const MIN_SCALE = 0.05
export const MAX_SCALE = 16
export const ZOOM_STEP = 1.25
export const WHEEL_STEP = 1.1

export interface Size {
  width: number
  height: number
}

export function clampScale(scale: number): number {
  return Math.min(Math.max(scale, MIN_SCALE), MAX_SCALE)
}

/** 회전(90° 단위)을 반영해 영역 안에 전부 들어오는 배율 — 확대는 하지 않는다(최대 1). 크기를 모르면 1. */
export function fitScale(box: Size, natural: Size | null, rotation: number): number {
  if (!natural || natural.width <= 0 || natural.height <= 0 || box.width <= 0 || box.height <= 0) return 1
  const turned = Math.abs(rotation) % 180 === 90
  const w = turned ? natural.height : natural.width
  const h = turned ? natural.width : natural.height
  return Math.min(1, box.width / w, box.height / h)
}

export interface Transform {
  scale: number
  tx: number
  ty: number
}

/** 영역 중심 기준 좌표 `(px, py)` 의 점이 제자리에 있도록 배율을 바꾼다. */
export function zoomAround(view: Transform, factor: number, px: number, py: number): Transform {
  const scale = clampScale(view.scale * factor)
  const ratio = scale / view.scale
  return { scale, tx: px - (px - view.tx) * ratio, ty: py - (py - view.ty) * ratio }
}
