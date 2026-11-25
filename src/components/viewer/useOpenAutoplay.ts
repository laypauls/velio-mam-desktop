import { createContext, useContext, useRef } from "react"
import { useSettings } from "@/hooks/useSettings"
import { usePlaybackStore } from "@/stores"

/**
 * 사용자가 재생을 시작해(포스터의 ▶ 등) 실제 플레이어를 띄운 경우 true — 설정과 무관하게 자동 재생으로 열어야 한다.
 * `ViewerMedia` 가 플레이어 트리를 이 값으로 감싼다.
 */
export const ForceAutoplayContext = createContext(false)

/**
 * 이 항목을 열 때 자동 재생할지 (D11). 컬렉션 연속 재생의 현재 항목은 설정과 무관하게 항상 자동 재생하고,
 * 그 밖에는 설정 `mediaOpenAutoplay` 를 따른다. 설정은 처음 알게 된 값으로 고정한다 — 재생 중 설정을 바꿔도 이미 열린 항목에는 영향이 없다.
 * 설정을 아직 못 받았으면 정지 상태(기본값)로 본다. `ForceAutoplayContext` 가 true 면(사용자가 시작함) 항상 true.
 */
export function useOpenAutoplay(assetId: number): boolean {
  const settings = useSettings()
  const forced = useContext(ForceAutoplayContext)
  const latched = useRef<boolean | null>(null)
  if (latched.current === null && (settings.data !== undefined || settings.isError)) latched.current = settings.data?.mediaOpenAutoplay === true
  const playingNow = usePlaybackStore((s) => s.status === "playing" && s.currentId === assetId)
  return forced || playingNow || latched.current === true
}
