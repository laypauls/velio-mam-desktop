import { useEffect, useRef } from "react"
import { normalizePlayerVolume, usePlayerVolumeStore } from "@/stores/playerVolume"
import { useSettings, useUpdateSettings } from "./useSettings"
import { useToastError } from "./useUserData"

/** 변경을 모아 저장하는 지연(ms) — 슬라이더 드래그·휠 중 연속 저장을 막는다 */
export const PLAYER_VOLUME_SAVE_DELAY_MS = 300

/**
 * 플레이어 음량·음소거 ↔ 설정(`playerVolume`) 동기화. 앱 셸에서 한 번 호출한다.
 * - 시작: 설정이 로드되면 한 번 채운다. 그 사이 사용자가 바꿨으면 사용자 값이 이긴다
 * - 변경: 마지막 변경 후 300ms 뒤 저장한다. 실패하면 toast, 로컬 값은 유지
 */
export function usePlayerVolumePersistence(): void {
  const settings = useSettings()
  const update = useUpdateSettings({ silent: true })
  const toastError = useToastError()
  const hydrate = usePlayerVolumeStore((s) => s.hydrate)
  const rev = usePlayerVolumeStore((s) => s.rev)
  const hydrated = useRef(false)
  const save = useRef({ mutate: update.mutate, toastError })
  save.current = { mutate: update.mutate, toastError }

  const stored = settings.data?.playerVolume
  const loaded = settings.data !== undefined
  useEffect(() => {
    if (hydrated.current || !loaded) return
    hydrated.current = true
    hydrate(normalizePlayerVolume(stored))
  }, [loaded, stored, hydrate])

  useEffect(() => {
    if (rev === 0) return
    const timer = setTimeout(() => {
      const { mutate, toastError: onError } = save.current
      const { level, muted } = usePlayerVolumeStore.getState()
      mutate({ playerVolume: { level, muted } }, { onError })
    }, PLAYER_VOLUME_SAVE_DELAY_MS)
    return () => clearTimeout(timer)
  }, [rev])
}
