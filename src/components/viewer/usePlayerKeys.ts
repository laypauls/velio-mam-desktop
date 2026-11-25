import { useEffect, useRef } from "react"
import { OPEN_LAYER, isEditableTarget } from "@/lib/openLayer"
import { playRangeKeyAction } from "./playRangeKeys"
import { playbackKeyAction, playerKeyAction, runPlayerAction, type PlayerKeyActions } from "./playerKeys"
import { sceneKeyAction } from "./sceneKeys"

/** 이 요소 안의 화살표는 그 요소가 쓴다 (슬라이더, 셀렉트) — 에디터블은 isEditableTarget 이 거른다 */
const OWN_KEYS = '[role="slider"], [role="combobox"]'
/** 플레이어 컨트롤 바(MediaControls 루트) — 이 안의 슬라이더·셀렉트에 포커스가 있어도 플레이어 단축키가 우선한다 (드래그 뒤 Space 가 죽지 않게) */
const PLAYER_CONTROLS = "[data-player-controls]"

/**
 * 열려 있는 플레이어(영상·음성)의 단축키 — Space · ←/→ · ↑/↓(Shift 와 함께면 시스템 음량) · `,`/`.` · M, 재생 구간 키(설정 프리셋), 영상의 장면전환점 키(Ctrl+←/→ 이동·C 추가),
 * 단독 보기의 이전/다음 콘텐츠(Shift+P/N, `contentNav` 가 있을 때만).
 * 포커스 위치와 무관하게(갤러리·상세 패널 등) 동작하도록 window 캡처 단계에서 처리하고,
 * preventDefault + stopPropagation 으로 갤러리의 같은 키 처리(커서 이동·Space 선택)가 겹쳐 실행되지 않게 한다.
 * 플레이어가 마운트된 동안만 리스너가 있다(이미지 뷰어·보기 영역 없음에서는 기존 동작 그대로).
 * 열린 Dialog·메뉴·입력 요소에서는 개입하지 않는다.
 */
export function usePlayerKeys(actions: PlayerKeyActions) {
  const live = useRef(actions)
  live.current = actions
  useEffect(() => {
    let swallowSpaceUp = false
    const onKeyDown = (e: KeyboardEvent) => {
      const a = live.current
      if (e.defaultPrevented || !a.ready) return
      const action = playerKeyAction(e, a.isVideo)
      const rangeAction = action === null && a.ranges ? playRangeKeyAction(e, a.ranges.preset) : null
      const sceneAction = action === null && rangeAction === null && a.isVideo && a.scenes ? sceneKeyAction(e) : null
      const navAction = action === null && rangeAction === null && sceneAction === null && a.contentNav ? playbackKeyAction(e) : null
      if (action === null && rangeAction === null && sceneAction === null && navAction === null) return
      // Delete 는 갤러리 등 다른 곳에서도 쓸 수 있는 키 — 선택된 구간이 있을 때만 가져간다
      if (rangeAction?.type === "deleteSelected" && !a.ranges?.hasSelection) return
      const target = e.target instanceof HTMLElement ? e.target : null
      const ownKeys = target?.closest(OWN_KEYS) && !target.closest(PLAYER_CONTROLS)
      if (isEditableTarget(e.target) || ownKeys || document.querySelector(OPEN_LAYER)) return
      e.preventDefault()
      e.stopPropagation()
      if (e.key === " ") swallowSpaceUp = true
      if (rangeAction !== null) {
        if (!e.repeat) a.ranges?.run(rangeAction.type)
        return
      }
      if (sceneAction !== null) {
        // 이동은 누르고 있으면 반복, 추가는 한 번만
        if (sceneAction === "add") {
          if (!e.repeat) a.scenes?.add()
        } else a.scenes?.move(sceneAction === "next" ? 1 : -1)
        return
      }
      if (navAction !== null) {
        // 갤러리 이동은 선택을 바꾸므로 누르고 있어도 한 번만, 끝(비활성)에서는 동작 없음
        const nav = a.contentNav
        if (e.repeat || !nav) return
        if (navAction === "next" ? nav.canNext : nav.canPrev) (navAction === "next" ? nav.onNext : nav.onPrev)()
        return
      }
      if (action === null) return
      if (e.repeat && action.type !== "seekBy" && action.type !== "volumeBy" && action.type !== "systemVolumeBy") return
      runPlayerAction(action, a)
    }
    // 포커스된 버튼의 Space 는 keyup 에서 클릭되므로, 처리한 Space 의 keyup 도 막아 이중 동작을 없앤다
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
