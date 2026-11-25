import { useCallback } from "react"
import { useViewStore } from "@/stores"

/** 전체화면 토글 — 현재 값은 호출 시점에 읽어 키 처리기가 낡은 값을 쥐지 않게 한다 */
export function useToggleFullscreen(): () => void {
  return useCallback(() => {
    const { fullscreen, setFullscreen } = useViewStore.getState()
    setFullscreen(!fullscreen)
  }, [])
}
