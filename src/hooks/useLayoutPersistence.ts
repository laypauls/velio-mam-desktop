import { useEffect, useRef } from "react"
import type { LayoutSettings } from "@/lib/ipc-contract"
import { FILMSTRIP_DEFAULT_HEIGHT, VIEWER_DEFAULT_WIDTH, VIEWER_MIN_WIDTH, clampFilmstripHeight, useViewStore, type DetailTab } from "@/stores/view"
import { useSettings, useUpdateSettings } from "./useSettings"
import { useToastError } from "./useUserData"

/** 변경을 모아 저장하는 지연(ms) — 폭 드래그 중 연속 저장을 막는다 */
export const LAYOUT_SAVE_DELAY_MS = 300

const TABS: readonly DetailTab[] = ["info", "ai", "similar"]

/** 저장값(누락·깨진 필드 포함 가능)을 필드별로 기본값으로 보정한다. 폭은 최소 320 */
export function normalizeLayout(raw: unknown): LayoutSettings {
  const v = (typeof raw === "object" && raw !== null ? raw : {}) as Partial<Record<keyof LayoutSettings, unknown>>
  const width = typeof v.viewerWidth === "number" && Number.isFinite(v.viewerWidth) ? Math.round(v.viewerWidth) : VIEWER_DEFAULT_WIDTH
  return {
    detailOpen: typeof v.detailOpen === "boolean" ? v.detailOpen : true,
    sidebarOpen: typeof v.sidebarOpen === "boolean" ? v.sidebarOpen : true,
    viewerWidth: Math.max(VIEWER_MIN_WIDTH, width),
    detailTab: TABS.includes(v.detailTab as DetailTab) ? (v.detailTab as DetailTab) : "info",
    filmstripHeight: typeof v.filmstripHeight === "number" && Number.isFinite(v.filmstripHeight) ? clampFilmstripHeight(v.filmstripHeight) : FILMSTRIP_DEFAULT_HEIGHT,
  }
}

/**
 * 레이아웃(상세 패널·사이드바 열림, 콘텐츠 보기 폭, 상세 탭) ↔ 설정(`layout`) 동기화. 앱 셸에서 한 번 호출한다.
 * - 시작: 설정이 로드되면 한 번 채운다(첫 렌더를 막지 않음). 그 사이 사용자가 바꿨으면 사용자 값이 이긴다
 * - 변경: 마지막 변경 후 300ms 뒤 전체 객체를 저장한다. 실패하면 toast, 로컬 값은 유지
 */
export function useLayoutPersistence(): void {
  const settings = useSettings()
  const update = useUpdateSettings({ silent: true })
  const toastError = useToastError()
  const hydrate = useViewStore((s) => s.hydrateLayout)
  const rev = useViewStore((s) => s.layoutRev)
  const hydrated = useRef(false)
  const save = useRef({ mutate: update.mutate, toastError })
  save.current = { mutate: update.mutate, toastError }

  const stored = settings.data?.layout
  const loaded = settings.data !== undefined
  useEffect(() => {
    if (hydrated.current || !loaded) return
    hydrated.current = true
    hydrate(normalizeLayout(stored))
  }, [loaded, stored, hydrate])

  useEffect(() => {
    if (rev === 0) return
    const timer = setTimeout(() => {
      const { mutate, toastError: onError } = save.current
      const { detailOpen, sidebarOpen, viewerWidth, detailTab, filmstripHeight } = useViewStore.getState()
      mutate({ layout: { detailOpen, sidebarOpen, viewerWidth, detailTab, filmstripHeight } }, { onError })
    }, LAYOUT_SAVE_DELAY_MS)
    return () => clearTimeout(timer)
  }, [rev])
}
