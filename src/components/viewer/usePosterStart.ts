import { useEffect, type RefObject } from "react"
import { OPEN_LAYER, isEditableTarget } from "@/lib/openLayer"

/**
 * 포스터(시작 전) 단계의 시작 키 — Space 는 포커스 위치와 무관하게(플레이어 단축키와 같은 규칙), Enter 는 포스터 영역 안에 포커스가 있을 때만.
 * 갤러리의 Enter(항목 열기)를 가로채지 않기 위해 Enter 는 `rootRef` 안의 대상으로 한정한다. 입력 요소·열린 대화상자/메뉴에서는 개입하지 않는다.
 */
export function usePosterStart(rootRef: RefObject<HTMLElement | null>, onStart: () => void) {
  useEffect(() => {
    let swallowSpaceUp = false
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.ctrlKey || e.altKey || e.shiftKey || e.metaKey) return
      const isSpace = e.key === " "
      const isEnter = e.key === "Enter" && e.target instanceof Node && rootRef.current?.contains(e.target) === true
      if (!isSpace && !isEnter) return
      if (isEditableTarget(e.target) || document.querySelector(OPEN_LAYER)) return
      e.preventDefault()
      e.stopPropagation()
      if (isSpace) swallowSpaceUp = true
      if (!e.repeat) onStart()
    }
    // 포커스된 버튼의 Space 는 keyup 에서 클릭되므로 처리한 Space 의 keyup 도 막는다 (이중 동작 방지)
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key !== " " || !swallowSpaceUp) return
      swallowSpaceUp = false
      e.preventDefault()
      e.stopPropagation()
    }
    window.addEventListener("keydown", onKeyDown, true)
    window.addEventListener("keyup", onKeyUp, true)
    return () => {
      window.removeEventListener("keydown", onKeyDown, true)
      window.removeEventListener("keyup", onKeyUp, true)
    }
  }, [rootRef, onStart])
}
