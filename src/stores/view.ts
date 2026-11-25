import { create } from "zustand"
import {
  DEFAULT_CARD_DISPLAY,
  PRESETS,
  type CardDisplayPatch,
  type CardPresetName,
} from "@/components/gallery/cardDisplay"
import type { CardSize, GalleryCardDisplay, LayoutSettings } from "@/lib/ipc-contract"

export const VIEWER_MIN_WIDTH = 320
export const VIEWER_DEFAULT_WIDTH = 520

export const FILMSTRIP_MIN_HEIGHT = 72
export const FILMSTRIP_MAX_HEIGHT = 160
export const FILMSTRIP_DEFAULT_HEIGHT = 104

/** 필름스트립 높이를 허용 범위(72~160)로 보정한다 — NaN 은 기본값 */
export function clampFilmstripHeight(height: number): number {
  if (!Number.isFinite(height)) return FILMSTRIP_DEFAULT_HEIGHT
  return Math.min(FILMSTRIP_MAX_HEIGHT, Math.max(FILMSTRIP_MIN_HEIGHT, Math.round(height)))
}

/** 갤러리 배치(격자/목록)와 필름스트립(보기가 갤러리 자리를 차지하고 갤러리는 한 줄 스트립) */
export type ViewMode = "grid" | "list" | "filmstrip"
/** 갤러리가 실제로 그리는 배치 — 필름스트립 중에도 숨은 갤러리는 직전 배치를 유지한다 */
export type GalleryLayoutMode = Exclude<ViewMode, "filmstrip">
export type DetailTab = "info" | "ai" | "similar"
export type SidebarTab = "library" | "collections"

interface ViewState {
  viewMode: ViewMode
  /** 필름스트립에서 돌아갈 직전 격자/목록 모드 — 필름스트립이 아닐 때는 `viewMode` 와 같다 */
  prevViewMode: GalleryLayoutMode
  /** 필름스트립 높이(px, 72~160) — 저장은 `useLayoutPersistence` 가 설정 `layout` 으로 보낸다 */
  filmstripHeight: number
  /** 필름스트립 모드에서 하단 스트립 표시 여부 — 세션 동안만 유지(저장 안 함) */
  stripVisible: boolean
  /** 갤러리 카드 표시 옵션 (격자 보기). 저장은 `useCardDisplayPersistence` 가 설정으로 보낸다 */
  cardDisplay: GalleryCardDisplay
  /** 사용자가 옵션을 바꾼 횟수 — 0 이면 아직 저장값으로 채우기 전/후 변경 없음. 저장 트리거로 쓴다 */
  cardDisplayRev: number
  /** 사용자가 레이아웃(상세·사이드바 열림, 보기 폭, 상세 탭)을 바꾼 횟수 — 0 이면 변경 없음. 저장 트리거이자 저장값 채우기 가드 */
  layoutRev: number
  /** 페이지 모드의 0-기준 쪽 번호 — 검색 조건이 바뀌면 갤러리가 0 으로 되돌린다 */
  galleryPage: number
  sidebarOpen: boolean
  /** 사이드바 탭 — 세션 동안만 유지(저장 안 함), 재시작하면 라이브러리 */
  sidebarTab: SidebarTab
  detailOpen: boolean
  /** 상세 패널 탭 — 자산을 바꿔도(로딩 중 포함) 유지한다 */
  detailTab: DetailTab
  /** 정보 탭 "세부 정보" 펼침 여부 — 세션 동안만 유지 */
  detailMetaOpen: boolean
  /** 콘텐츠 보기 영역이 보여주는 자산 (null 이면 영역을 접는다) */
  previewId: number | null
  /** 콘텐츠 보기 영역 폭(px) — 저장은 `useLayoutPersistence` 가 설정 `layout` 으로 보낸다 */
  viewerWidth: number
  /** 큰 보기 플레이어가 이동할 시각(ms) — `usePreviewSeek` 이 소비한다. `seekTick` 은 같은 시각 재요청을 구분한다 */
  seekMs: number | null
  seekTick: number
  /** 보기 영역 전체화면 — 창 전체화면 적용은 `useWindowFullscreen` 이 한다. 보기 영역이 닫히면 항상 false */
  fullscreen: boolean
  /** 모드 전환 — 필름스트립을 벗어나면 분할 보기 영역도 닫는다(D2). 필름스트립에 들어가면 직전 격자/목록을 기억한다 */
  setViewMode: (m: ViewMode) => void
  setFilmstripHeight: (height: number) => void
  setStripVisible: (visible: boolean) => void
  setGalleryPage: (page: number) => void
  /** 카드 크기 단계 — `cardDisplay.cardSize` 를 바꾸고 표시 옵션과 같은 저장 경로를 탄다 */
  setCardSize: (size: CardSize) => void
  setCardDisplay: (patch: CardDisplayPatch) => void
  applyCardPreset: (name: CardPresetName) => void
  /** 표시 옵션만 기본값으로 — 카드 크기는 유지한다 */
  resetCardDisplay: () => void
  /** 저장된 값으로 채운다 — 사용자가 이미 바꿨으면(rev > 0) 무시한다 */
  hydrateCardDisplay: (display: GalleryCardDisplay) => void
  /** 저장된 레이아웃으로 채운다 — 사용자가 이미 바꿨으면(layoutRev > 0) 무시한다. 저장을 유발하지 않는다 */
  hydrateLayout: (layout: LayoutSettings) => void
  toggleSidebar: () => void
  setSidebarTab: (tab: SidebarTab) => void
  setDetailOpen: (open: boolean) => void
  setDetailTab: (tab: DetailTab) => void
  setDetailMetaOpen: (open: boolean) => void
  setViewerWidth: (width: number) => void
  openPreview: (id: number, seekMs?: number) => void
  /** 이미 열린 큰 보기면 그 자리에서 이동하고, 아니면 해당 시각으로 연다 */
  seekPreview: (id: number, ms: number) => void
  closePreview: () => void
  setFullscreen: (on: boolean) => void
}

export const useViewStore = create<ViewState>()((set) => ({
  viewMode: "grid",
  prevViewMode: "grid",
  filmstripHeight: FILMSTRIP_DEFAULT_HEIGHT,
  stripVisible: true,
  cardDisplay: DEFAULT_CARD_DISPLAY,
  cardDisplayRev: 0,
  layoutRev: 0,
  galleryPage: 0,
  sidebarOpen: true,
  sidebarTab: "library",
  detailOpen: true,
  detailTab: "info",
  detailMetaOpen: false,
  previewId: null,
  viewerWidth: VIEWER_DEFAULT_WIDTH,
  seekMs: null,
  seekTick: 0,
  fullscreen: false,
  setViewMode: (viewMode) =>
    set((s) => {
      if (viewMode === s.viewMode) return s
      if (viewMode === "filmstrip") return { viewMode, prevViewMode: s.viewMode as GalleryLayoutMode }
      // 필름스트립을 떠난다 — 보기 영역은 닫고(재생도 멈춘다), 갤러리는 현재 항목으로 스크롤된다
      if (s.viewMode === "filmstrip") return { viewMode, prevViewMode: viewMode, previewId: null, seekMs: null, fullscreen: false }
      return { viewMode, prevViewMode: viewMode }
    }),
  setFilmstripHeight: (height) =>
    set((s) => {
      const filmstripHeight = clampFilmstripHeight(height)
      return filmstripHeight === s.filmstripHeight ? s : { filmstripHeight, layoutRev: s.layoutRev + 1 }
    }),
  setStripVisible: (stripVisible) => set({ stripVisible }),
  setGalleryPage: (galleryPage) => set({ galleryPage: Math.max(0, Math.floor(galleryPage)) }),
  setCardDisplay: (patch) =>
    set((s) => ({
      cardDisplay: { ...s.cardDisplay, ...patch, info: { ...s.cardDisplay.info, ...patch.info } },
      cardDisplayRev: s.cardDisplayRev + 1,
    })),
  setCardSize: (cardSize) => set((s) => ({ cardDisplay: { ...s.cardDisplay, cardSize }, cardDisplayRev: s.cardDisplayRev + 1 })),
  applyCardPreset: (name) =>
    set((s) => ({ cardDisplay: { ...s.cardDisplay, info: { ...PRESETS[name] } }, cardDisplayRev: s.cardDisplayRev + 1 })),
  resetCardDisplay: () =>
    set((s) => ({ cardDisplay: { ...structuredClone(DEFAULT_CARD_DISPLAY), cardSize: s.cardDisplay.cardSize }, cardDisplayRev: s.cardDisplayRev + 1 })),
  hydrateCardDisplay: (cardDisplay) => set((s) => (s.cardDisplayRev === 0 ? { cardDisplay } : s)),
  hydrateLayout: (l) => set((s) => (s.layoutRev === 0 ? { detailOpen: l.detailOpen, sidebarOpen: l.sidebarOpen, viewerWidth: l.viewerWidth, detailTab: l.detailTab, filmstripHeight: clampFilmstripHeight(l.filmstripHeight) } : s)),
  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen, layoutRev: s.layoutRev + 1 })),
  setSidebarTab: (sidebarTab) => set({ sidebarTab }),
  setDetailOpen: (detailOpen) => set((s) => (s.detailOpen === detailOpen ? s : { detailOpen, layoutRev: s.layoutRev + 1 })),
  setDetailTab: (detailTab) => set((s) => (s.detailTab === detailTab ? s : { detailTab, layoutRev: s.layoutRev + 1 })),
  setDetailMetaOpen: (detailMetaOpen) => set({ detailMetaOpen }),
  setViewerWidth: (width) =>
    set((s) => {
      const viewerWidth = Math.max(VIEWER_MIN_WIDTH, Math.round(width))
      return viewerWidth === s.viewerWidth ? s : { viewerWidth, layoutRev: s.layoutRev + 1 }
    }),
  openPreview: (previewId, seekMs) => set((s) => ({ previewId, seekMs: seekMs ?? null, seekTick: s.seekTick + 1 })),
  seekPreview: (previewId, seekMs) => set((s) => ({ previewId, seekMs, seekTick: s.seekTick + 1 })),
  // 필름스트립에서는 보기가 곧 모드라 닫기(Esc·Q·✕)가 직전 격자/목록으로 돌아간다
  closePreview: () =>
    set((s) => ({ previewId: null, seekMs: null, fullscreen: false, ...(s.viewMode === "filmstrip" ? { viewMode: s.prevViewMode } : {}) })),
  // 열린 보기 영역이 없으면 전체화면으로 만들 수 없다
  setFullscreen: (fullscreen) => set((s) => (s.fullscreen === fullscreen || (fullscreen && s.previewId === null) ? s : { fullscreen })),
}))
