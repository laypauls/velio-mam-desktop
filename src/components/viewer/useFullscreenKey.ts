import { useEffect } from "react"
import { OPEN_LAYER, isEditableTarget } from "@/lib/openLayer"
import { useToggleFullscreen } from "./useToggleFullscreen"

/** 이미지 보기의 `F`(물리 키 — 한글 IME 에서도) 전체화면 토글. 플레이어는 usePlayerKeys 가 같은 키를 처리한다. 입력 요소·열린 레이어·수정키 조합·키 반복은 무시 */
export function useFullscreenKey() {
  const toggle = useToggleFullscreen()
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.code !== "KeyF" || e.defaultPrevented || e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return
      if (isEditableTarget(e.target) || document.querySelector(OPEN_LAYER)) return
      e.preventDefault()
      if (!e.repeat) toggle()
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [toggle])
}
