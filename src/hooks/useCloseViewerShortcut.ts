import { useEffect, useRef } from "react"
import { OPEN_LAYER, isEditableTarget } from "@/lib/openLayer"
import { matchesShortcut } from "@/lib/shortcut"
import { useViewStore } from "@/stores"
import { useSettings } from "./useSettings"

/**
 * "플레이어 패널 닫기" 단축키 (설정 `closeViewerShortcut`, 기본 Q) — 콘텐츠 뷰어가 마운트된 동안만 등록한다.
 * ✕ 버튼과 같은 `closePreview` 경로라 재생도 함께 멈춘다. 입력 요소·열린 Dialog/메뉴/Select 에서는 개입하지 않고 키 반복은 무시한다.
 * window **버블** 단계 — 플레이어 키(캡처)와 숨기기 단축키(캡처)가 이미 처리한 키는 `defaultPrevented` 로 건너뛰고,
 * 숨기기 단축키와 같은 키로 저장돼 있어도(UI 는 거부) 숨기기가 이긴다.
 */
export function useCloseViewerShortcut() {
  const settings = useSettings().data
  const live = useRef({ shortcut: null as string | null, hide: null as string | null })
  live.current = { shortcut: settings?.closeViewerShortcut ?? null, hide: settings?.hideShortcut ?? null }
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const { shortcut, hide } = live.current
      if (e.defaultPrevented || shortcut === null || shortcut === hide || !matchesShortcut(e, shortcut)) return
      if (isEditableTarget(e.target) || document.querySelector(OPEN_LAYER)) return
      e.preventDefault()
      if (e.repeat) return
      useViewStore.getState().closePreview()
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [])
}
