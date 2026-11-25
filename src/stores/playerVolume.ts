import { create } from "zustand"
import type { PlayerVolumeSettings } from "@/lib/ipc-contract"

export const DEFAULT_PLAYER_VOLUME: PlayerVolumeSettings = { level: 100, muted: false }

/** 음량(0~100 정수)으로 맞춘다 — 범위 밖은 clamp, 소수는 반올림, 숫자가 아니면 null */
function toLevel(raw: unknown): number | null {
  return typeof raw === "number" && Number.isFinite(raw) ? Math.min(100, Math.max(0, Math.round(raw))) : null
}

/** 저장값(누락·깨진 필드 포함 가능)을 필드별로 기본값으로 보정한다 */
export function normalizePlayerVolume(raw: unknown): PlayerVolumeSettings {
  const v = (typeof raw === "object" && raw !== null ? raw : {}) as Partial<Record<keyof PlayerVolumeSettings, unknown>>
  return {
    level: toLevel(v.level) ?? DEFAULT_PLAYER_VOLUME.level,
    muted: typeof v.muted === "boolean" ? v.muted : DEFAULT_PLAYER_VOLUME.muted,
  }
}

interface PlayerVolumeState extends PlayerVolumeSettings {
  /** 사용자가 바꾼 횟수 — 0 이면 아직 저장값으로 채울 수 있고, 증가하면 `usePlayerVolumePersistence` 가 저장한다 */
  rev: number
  /** 음량 변경(0~100 으로 보정). 0 보다 크면 음소거를 해제한다. 0 은 음소거와 별개로 저장한다 */
  setLevel: (level: number) => void
  setMuted: (muted: boolean) => void
  /** 저장된 값으로 채운다 — 사용자가 이미 바꿨으면(rev > 0) 무시한다. 저장을 유발하지 않는다 */
  hydrate: (v: PlayerVolumeSettings) => void
}

/** 두 재생 경로(`<video>`·libmpv)가 공유하는 음량·음소거. 파일을 바꿔도 유지된다 */
export const usePlayerVolumeStore = create<PlayerVolumeState>((set) => ({
  ...DEFAULT_PLAYER_VOLUME,
  rev: 0,
  setLevel: (raw) =>
    set((s) => {
      const level = toLevel(raw)
      if (level === null) return s
      const muted = level > 0 ? false : s.muted
      return level === s.level && muted === s.muted ? s : { level, muted, rev: s.rev + 1 }
    }),
  setMuted: (muted) => set((s) => (s.muted === muted ? s : { muted, rev: s.rev + 1 })),
  hydrate: (v) => set((s) => (s.rev === 0 ? { level: v.level, muted: v.muted } : s)),
}))
