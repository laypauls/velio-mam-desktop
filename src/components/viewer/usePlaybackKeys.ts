import { useEffect } from "react"
import { OPEN_LAYER, isEditableTarget } from "@/lib/openLayer"
import { usePlaybackStore } from "@/stores"
import { playbackKeyAction } from "./playerKeys"

/** Space 로 일시정지/재개할 수 있는 상태 — 현재 항목이 이미지일 때만 (영상·음성은 usePlayerKeys 가 Space 를 처리한다) */
function imagePausable(): boolean {
  const p = usePlaybackStore.getState()
  return (p.status === "playing" || p.status === "paused") && p.queue[p.index]?.mediaType === "image"
}

/**
 * 연속 재생 중 Shift+N(다음) / Shift+P(이전), 이미지 항목에서 Space(슬라이드쇼 일시정지/재개) —
 * 포커스 위치와 무관하게 window 캡처 단계에서 처리한다. 입력 요소·열린 Dialog/메뉴에서는 개입하지 않는다.
 * 재생이 활성인 동안(`PlaybackBar` 마운트)만 리스너가 있다. 이미지 Space 는 갤러리의 Space(선택 토글)보다 우선한다.
 */
export function usePlaybackKeys() {
  useEffect(() => {
    let swallowSpaceUp = false
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return
      const isSpace = e.key === " " && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey
      const action = isSpace ? null : playbackKeyAction(e)
      if (!isSpace && action === null) return
      if (isEditableTarget(e.target) || document.querySelector(OPEN_LAYER)) return
      if (isSpace && !imagePausable()) return
      e.preventDefault()
      e.stopPropagation()
      if (isSpace) swallowSpaceUp = true
      if (e.repeat) return
      const p = usePlaybackStore.getState()
      if (isSpace) p.togglePause()
      else if (action === "next") p.next()
      else p.prev()
    }
    // 포커스된 버튼의 Space 는 keyup 에서 클릭되므로, 처리한 Space 의 keyup 도 막는다
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
  }, [])
}
