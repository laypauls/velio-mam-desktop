/** 장면전환점 키 — 재생 구간 프리셋과 무관한 고정 키. 이동 `Ctrl+←/→`, 추가 `C`(물리 키). 영상에서만 쓴다 */
export type SceneKeyAction = "prev" | "next" | "add"

type KeyLike = { key: string; code: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }

/** usePlayerKeys 가 장면 키를 넘기는 대상 — 컨트롤러가 채운다 */
export interface SceneKeyHandlers {
  /** 이전/다음 장면전환점으로 이동 (마지막 컷 뒤·첫 컷 앞이면 동작 없음) */
  move: (dir: 1 | -1) => void
  /** 현재 위치를 사용자 컷으로 추가 */
  add: () => void
}

/**
 * 키 입력 → 장면 동작. 수정자는 정확히 일치해야 한다 — `Ctrl+←/→` 는 Ctrl 만(Shift·Alt·Meta 가 같이 눌리면 null),
 * `C` 는 수정자 없이 물리 키(`code === "KeyC"`)로만 판정해 한글 입력 상태("ㅊ")에서도 동작하고 Ctrl+C(복사)는 건드리지 않는다.
 */
export function sceneKeyAction(e: KeyLike): SceneKeyAction | null {
  if (e.metaKey || e.altKey) return null
  if (e.ctrlKey) {
    if (e.shiftKey) return null
    return e.key === "ArrowLeft" ? "prev" : e.key === "ArrowRight" ? "next" : null
  }
  return !e.shiftKey && e.code === "KeyC" ? "add" : null
}

const validFps = (fps: number | null): fps is number => fps !== null && Number.isFinite(fps) && fps > 0

/** 한 프레임 길이(ms) — fps 없으면 40 (반 프레임 20ms 와 짝) */
export function frameMs(fps: number | null): number {
  return validFps(fps) ? Math.max(1, Math.round(1000 / fps)) : 40
}

/** 컷 이동 때 더하는 반 프레임(ms) — 컷 시각 그대로 맞추면 반올림으로 이전 장면의 마지막 프레임이 보일 수 있다 (서버 `half_frame_ms` 와 같은 표) */
export function halfFrameMs(fps: number | null): number {
  return validFps(fps) ? Math.max(1, Math.round(500 / fps)) : 20
}

/** 마지막 이동 목표를 현재 위치 대신 기준으로 삼는 시간(ms) — 플레이어가 보고하는 시각은 최대 5Hz 로 갱신돼 연타하면 낡은 값이다 */
export const SCENE_NAV_BASE_WINDOW_MS = 1000

/**
 * 다음/이전 장면전환점의 이동 목표(ms, 컷 + 반 프레임) — 없으면 null.
 * `cuts` 는 오름차순. next = `min{x : x+hf > base+1}`, prev = `max{x : x+hf < base−프레임}` — 방금 이동한 위치(x+hf)는 다음/이전 후보에서 빠진다.
 */
export function sceneNav(cuts: readonly number[], baseMs: number, dir: 1 | -1, fps: number | null): number | null {
  const hf = halfFrameMs(fps)
  if (dir === 1) {
    const next = cuts.find((x) => x + hf > baseMs + 1)
    return next === undefined ? null : next + hf
  }
  const limit = baseMs - frameMs(fps)
  for (let i = cuts.length - 1; i >= 0; i--) if (cuts[i] + hf < limit) return cuts[i] + hf
  return null
}
