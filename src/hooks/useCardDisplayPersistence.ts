import { useEffect, useRef } from "react"
import { normalize } from "@/components/gallery/cardDisplay"
import { useViewStore } from "@/stores"
import { useSettings, useUpdateSettings } from "./useSettings"
import { useToastError } from "./useUserData"

/** 옵션 변경을 모아 저장하는 지연(ms) — 체크를 연달아 누를 때마다 쓰지 않도록 */
export const CARD_DISPLAY_SAVE_DELAY_MS = 300

/**
 * 갤러리 카드 표시 옵션 ↔ 설정(`galleryCardDisplay`) 동기화. 앱 셸에서 한 번 호출한다.
 * - 시작: 설정이 로드되면 한 번 채운다(첫 렌더를 막지 않음 — 그 전에는 기본값). 그 사이 사용자가 바꿨으면 사용자 값이 이긴다
 * - 변경: 마지막 변경 후 300ms 뒤 전체 객체를 저장한다(메뉴 닫힘과 무관하게 Esc·바깥 클릭에서도 저장됨). 실패하면 toast, 로컬 값은 유지
 */
export function useCardDisplayPersistence(): void {
  const settings = useSettings()
  const update = useUpdateSettings({ silent: true })
  const toastError = useToastError()
  const hydrate = useViewStore((s) => s.hydrateCardDisplay)
  const rev = useViewStore((s) => s.cardDisplayRev)
  const hydrated = useRef(false)
  const save = useRef({ mutate: update.mutate, toastError })
  save.current = { mutate: update.mutate, toastError }

  const stored = settings.data?.galleryCardDisplay
  const loaded = settings.data !== undefined
  useEffect(() => {
    if (hydrated.current || !loaded) return
    hydrated.current = true
    hydrate(normalize(stored))
  }, [loaded, stored, hydrate])

  useEffect(() => {
    if (rev === 0) return
    const timer = setTimeout(() => {
      const { mutate, toastError: onError } = save.current
      mutate({ galleryCardDisplay: useViewStore.getState().cardDisplay }, { onError })
    }, CARD_DISPLAY_SAVE_DELAY_MS)
    return () => clearTimeout(timer)
  }, [rev])
}
