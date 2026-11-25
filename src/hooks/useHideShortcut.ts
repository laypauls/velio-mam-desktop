import { useEffect, useRef } from "react"
import { useApi } from "@/lib/api"
import { OPEN_LAYER, isEditableTarget } from "@/lib/openLayer"
import { matchesShortcut } from "@/lib/shortcut"
import { useViewStore } from "@/stores"
import { useSettings } from "./useSettings"
import { useToastError } from "./useUserData"

/** 문서의 모든 `<video>`/`<audio>` 를 멈춘다 (libVLC 는 `hide_main_window` 가 코어에서 멈춘다) */
export function pauseAllMedia() {
  document.querySelectorAll<HTMLMediaElement>("video, audio").forEach((m) => {
    if (!m.paused) m.pause()
  })
}

/**
 * 앱 창에 포커스가 있을 때만 동작하는 "재생 정지 후 숨기기" 단축키 (설정 `hideShortcut`, 기본 Esc).
 * window **캡처 단계** 리스너라서 기존 Esc 처리기(ContentViewer·SettingsPage 의 window 버블 리스너, Gallery 의 React onKeyDown)보다
 * 먼저 실행되고, `preventDefault` + `stopPropagation` 으로 그것들이 중복 실행되지 않게 한다.
 * 열린 Dialog·메뉴·Select 나 입력 요소에서는 개입하지 않는다(그쪽 기존 동작). 복원 시 자동 재생하지 않는다.
 * 플레이어 전체화면 중에는 먼저 전체화면을 해제한 뒤 숨긴다. 단, Esc 는 "전체화면 해제만"(숨기지 않음)이다 — 기본 숨기기 키가 Esc 라서 전체화면 Esc 한 번에 앱이 사라지면 안 된다.
 */
export function useHideShortcut() {
  const api = useApi()
  const toastError = useToastError()
  const shortcut = useSettings().data?.hideShortcut ?? null
  const live = useRef({ shortcut, hide: () => {} })
  live.current = {
    shortcut,
    hide: () => {
      pauseAllMedia()
      api.hideMainWindow().catch(toastError)
    },
  }
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!matchesShortcut(e, live.current.shortcut)) return
      if (isEditableTarget(e.target) || document.querySelector(OPEN_LAYER)) return
      e.preventDefault()
      e.stopImmediatePropagation()
      if (e.repeat) return
      const view = useViewStore.getState()
      if (view.fullscreen) {
        view.setFullscreen(false)
        if (e.key === "Escape") return
      }
      live.current.hide()
    }
    window.addEventListener("keydown", onKeyDown, true)
    return () => window.removeEventListener("keydown", onKeyDown, true)
  }, [])
}
