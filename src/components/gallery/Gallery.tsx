import { DatabaseZap } from "lucide-react"
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { useOpenAsset, useOpenPreview } from "@/hooks/useOpenAsset"
import { useSettings } from "@/hooks/useSettings"
import { FilmstripView } from "@/components/viewer/filmstrip/FilmstripView"
import { DEFAULT_CAPTURED_AT_FALLBACK } from "@/lib/capturedAt"
import type { AssetSummary, CapturedAtFallback, GalleryPaging, IndexRailMode } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { DEFAULT_FILTERS, usePlaybackStore, useSelectionStore, useViewStore } from "@/stores"
import type { ClickMods } from "@/stores/selection"
import { AssetContextMenu } from "./AssetContextMenu"
import { CollectionHeader } from "./CollectionHeader"
import { DegradedNotice } from "./DegradedNotice"
import { EmptyState } from "./EmptyState"
import { GalleryPager } from "./GalleryPager"
import { GalleryEmpty, LoadingSkeleton } from "./GalleryStates"
import { IndexRail } from "./IndexRail"
import { InlineError } from "./InlineError"
import { SelectionBar } from "./SelectionBar"
import { optionId } from "./optionId"
import { useGalleryKeyboard, type PageEdge, type PageLanding } from "./useGalleryKeyboard"
import { useGalleryViewport, type GalleryScrollController } from "./useGalleryViewport"
import { usePublishViewerNav } from "./usePublishViewerNav"
import { ReorderActionsContext, useReorderActions } from "./useReorderActions"
import { useReorderDrag } from "./useReorderDrag"
import { VirtualGrid } from "./VirtualGrid"
import { VirtualList } from "./VirtualList"

export type { GalleryScrollController }

interface BodyProps {
  paging: GalleryPaging
  pageSize: number
  indexRail: IndexRailMode
  titleEditEnabled: boolean
  capturedAtFallback: CapturedAtFallback
  renderRail?: (controller: GalleryScrollController) => ReactNode
}

function GalleryBody({ paging, pageSize, indexRail, titleEditEnabled, capturedAtFallback, renderRail }: BodyProps) {
  const { t } = useTranslation()
  // 필름스트립 중에도 숨은 갤러리는 직전 격자/목록 배치를 유지한다 (돌아왔을 때 스크롤 위치·열 수 그대로)
  const viewMode = useViewStore((s) => (s.viewMode === "filmstrip" ? s.prevViewMode : s.viewMode))
  const filmstrip = useViewStore((s) => s.viewMode === "filmstrip")
  const previewId = useViewStore((s) => s.previewId)
  const openPreview = useOpenPreview()
  const selected = useSelectionStore((s) => s.ids)
  const selectAll = useSelectionStore((s) => s.selectAll)
  const clearSelection = useSelectionStore((s) => s.clear)
  const openAsset = useOpenAsset()
  const scrollAreaId = useId() // 목차 레일의 aria-controls
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null)
  const vp = useGalleryViewport({ el: scrollEl, paging, pageSize })
  const { win, geo, scroller } = vp
  const { getItem, positionOf } = win

  // 메모리에 있는 항목만 — Shift 범위·Ctrl+A(무한 스크롤)의 기준
  const order = useMemo(() => win.loadedIds(), [win.loadedIds, win.version])
  const lookup = useCallback(
    (id: number): AssetSummary | undefined => {
      const pos = positionOf(id)
      return pos === null ? undefined : getItem(pos)
    },
    [positionOf, getItem]
  )
  // 더블클릭·Enter 공통 열기 — 설정(미리보기/기본 프로그램)을 따르고, 오프라인·파일 없음이면 열지 않는다
  const onOpen = useCallback((id: number) => openAsset(id, lookup(id)?.availability ?? "online"), [openAsset, lookup])

  // 카드 빠른 동작 "크게 보기" — 오프라인·파일 없음이면 열지 않고 사유를 알린다
  const onPreview = useCallback((id: number) => openPreview(id, lookup(id)?.availability ?? "online"), [openPreview, lookup])

  // 컬렉션 순서 이동 — 메뉴·단축키·드래그가 같은 동작을 쓴다
  // 메뉴·단축키 이동 뒤 이동한 덩어리를 따라간다 — 현재 쪽 안이면 최소 이동, 다른 쪽이면 그 쪽으로 (비동기 호출이라 최신 값을 ref 로 읽는다)
  const followRef = useRef<(pos: number) => void>(() => undefined)
  followRef.current = (pos) => (pos >= win.range[0] && pos < win.range[1] ? vp.reveal(pos) : vp.controller.scrollToItem(pos))
  const follow = useCallback((pos: number) => followRef.current(pos), [])
  const reorder = useReorderActions({ getItem, positionOf, total: win.total, request: win.request, follow })
  const drag = useReorderDrag({ reorder, layout: viewMode, scrollBy: scroller.scrollBy, viewport: scrollEl })
  const showOrder = win.shownSortField === "collectionOrder"

  const landOnRef = useRef<(pos: number) => void>(() => undefined)
  const pageEdge = useMemo<PageEdge | undefined>(
    () =>
      paging === "paged"
        ? {
            canStep: (dir) => (dir > 0 ? win.page < win.pageCount - 1 : win.page > 0),
            step: (dir: 1 | -1, landing: PageLanding) => {
              const next = win.page + dir
              vp.goToPage(next, landing)
              landOnRef.current(landing === "first" ? next * pageSize : Math.min(win.total, (next + 1) * pageSize) - 1)
            },
          }
        : undefined,
    [paging, pageSize, win.page, win.pageCount, win.total, vp.goToPage]
  )
  const kbd = useGalleryKeyboard({ range: win.range, nav: vp.nav, getItem, positionOf, order, reveal: vp.reveal, pageEdge, onOpen, reorder })
  landOnRef.current = kbd.landOn
  usePublishViewerNav(kbd.viewerStep)

  // 필름스트립: Home/End — 페이지 모드에서 다른 쪽이면 그 쪽으로 옮긴 뒤 항목에 놓는다
  const onJump = (edge: "first" | "last") => {
    if (win.total <= 0) return
    const pos = edge === "first" ? 0 : win.total - 1
    if (paging === "paged" && (pos < win.range[0] || pos >= win.range[1])) {
      vp.goToPage(Math.floor(pos / pageSize), edge)
      landOnRef.current(pos)
    } else kbd.jumpTo(pos)
  }
  const stripPageStep = useMemo(
    () => (pageEdge ? { canStep: pageEdge.canStep, step: (dir: 1 | -1) => vp.goToPage(win.page + dir) } : undefined),
    [pageEdge, vp.goToPage, win.page]
  )
  // 필름스트립을 떠나면 현재 항목이 보이게 갤러리를 스크롤하고 포커스를 돌려준다
  const lastCurrent = useRef<number | null>(null)
  const wasFilmstrip = useRef(false)
  if (filmstrip && previewId !== null) lastCurrent.current = previewId
  useLayoutEffect(() => {
    if (wasFilmstrip.current && !filmstrip) {
      const id = lastCurrent.current ?? useSelectionStore.getState().activeId
      const pos = id !== null ? positionOf(id) : null
      if (pos !== null) {
        if (pos >= win.range[0] && pos < win.range[1]) vp.reveal(pos)
        else vp.controller.scrollToItem(pos)
      }
      scrollEl?.focus({ preventScroll: true })
    }
    wasFilmstrip.current = filmstrip
  })
  // 연속 재생 중에는 평범한 클릭이 대기열 항목으로 이동시킨다 (Ctrl/Shift 클릭·대기열 밖 카드는 선택만)
  const { onSelectWithCursor } = kbd
  const onSelect = useCallback(
    (id: number, mods: ClickMods) => {
      onSelectWithCursor(id, mods)
      usePlaybackStore.getState().clickAsset(id, mods)
    },
    [onSelectWithCursor]
  )

  // 검색 조건이 바뀌면 선택과 키보드 포커스 항목을 비운다 (이전 결과의 id 가 남지 않게)
  useEffect(() => {
    clearSelection()
    kbd.reset()
  }, [win.request, clearSelection])

  /** 페이지 모드: 이 쪽 전체 / 무한 스크롤: 메모리에 불러온 항목만 (선택 바가 안내한다) */
  const selectEverything = () => {
    if (paging === "paged") {
      const ids: number[] = []
      for (let g = win.range[0]; g < win.range[1]; g++) {
        const item = getItem(g)
        if (item) ids.push(item.assetId)
      }
      selectAll(ids)
    } else selectAll(order, { loadedOnly: win.total > order.length })
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (kbd.onKeyDown(e)) return
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault()
      selectEverything()
    } else if (e.key === "Escape" && !e.defaultPrevented) {
      // 콘텐츠 보기 영역이 열려 있으면 Esc 는 선택을 유지한 채 그것부터 닫는다
      if (useViewStore.getState().previewId !== null) {
        e.preventDefault()
        useViewStore.getState().closePreview()
      } else clearSelection()
    }
  }

  const filtered = win.request.query !== "" || JSON.stringify(win.request.filters) !== JSON.stringify(DEFAULT_FILTERS)
  const hasItems = win.total > 0

  // 결과가 없을 때의 상태 화면 (불러오는 중·색인 준비·오류·빈 결과) — 필름스트립도 같은 화면을 쓴다
  let stateContent: ReactNode = null
  if (win.isLoading && !hasItems) stateContent = <LoadingSkeleton width={vp.viewportW} />
  else if (win.indexPreparing && !hasItems)
    stateContent = <EmptyState icon={DatabaseZap} busy title={t("gallery.indexing.title")} description={t("gallery.indexing.description")} />
  else if (win.isError && !hasItems) stateContent = <InlineError error={win.error} onRetry={win.retry} />
  else if (!hasItems) stateContent = <GalleryEmpty filtered={filtered} />

  let content: ReactNode = stateContent
  if (stateContent === null) {
    const common = { geo, offset: scroller.offset, getItem, selected, activeId: kbd.cursorId, onSelect, onOpen, titleEditEnabled, capturedAtFallback, showOrder, draggable: reorder.enabled }
    content = (
      <AssetContextMenu lookup={lookup} className="h-full w-full" drag={drag}>
        {viewMode === "grid" ? <VirtualGrid {...common} onPreview={onPreview} /> : <VirtualList {...common} />}
      </AssetContextMenu>
    )
  }

  return (
    <ReorderActionsContext.Provider value={reorder}>
      <section aria-label={t("gallery.section")} className="relative flex min-h-0 min-w-0 flex-1 flex-col">
        {/* 필름스트립 중에는 갤러리를 가리되(invisible — 크기·스크롤 유지) 포커스·접근성에서 뺀다. 설정 화면이 라이브러리를 가리는 방식과 같다 */}
        <div inert={filmstrip} aria-hidden={filmstrip || undefined} className={cn("flex min-h-0 min-w-0 flex-1 flex-col", filmstrip && "invisible")}>
          <CollectionHeader />
          <SelectionBar lookup={lookup} />
          {win.degraded && win.request.mode === "hybrid" && <DegradedNotice />}
          {win.isError && hasItems && <InlineError error={win.error} onRetry={win.retry} />}
          <div className="flex min-h-0 flex-1">
            {/* 네이티브 스크롤바 없음 — 오프셋을 직접 관리한다(휠·키보드·레일). 콘텐츠 높이가 레이아웃 한계를 넘을 수 있어서다 */}
            <div
              ref={setScrollEl}
              id={scrollAreaId}
              tabIndex={0}
              onKeyDown={onKeyDown}
              role={hasItems ? "listbox" : undefined}
              aria-multiselectable={hasItems ? true : undefined}
              aria-busy={win.isRefreshing || undefined}
              aria-label={hasItems ? t("gallery.label") : t("gallery.scroll")}
              aria-activedescendant={hasItems && kbd.cursorId !== null ? optionId(kbd.cursorId) : undefined}
              // 컨테이너 전체 포커스 링 없음: 마우스 클릭 뒤 Esc·화살표 키로 :focus-visible 이 켜지면 갤러리 전체에 링이 그려진다. 키보드 위치는 커서 카드(data-active 외곽선)가 표시한다
              className="relative min-h-0 min-w-0 flex-1 overflow-x-auto overflow-y-hidden outline-none"
            >
              {content}
            </div>
            {hasItems && (renderRail ? renderRail(vp.controller) : <IndexRail controller={vp.controller} mode={indexRail} pageSize={pageSize} scrollAreaId={scrollAreaId} />)}
          </div>
          {paging === "paged" && hasItems && (
            <GalleryPager page={win.page} pageCount={win.pageCount} range={win.range} total={win.total} totalIsEstimate={win.totalIsEstimate} onPage={(p) => vp.goToPage(p)} />
          )}
        </div>
        {filmstrip && (
          <div className="absolute inset-0 flex">
            <FilmstripView
              source={{ total: win.total, range: win.range, getItem, requestRange: win.requestRange }}
              positionOf={positionOf}
              request={win.request}
              refreshing={win.isRefreshing}
              selected={selected}
              titleEditEnabled={titleEditEnabled}
              stepper={kbd.viewerStep}
              onSelect={onSelect}
              onJump={onJump}
              onForwardKey={kbd.onKeyDown}
              pageStep={stripPageStep}
              state={stateContent}
            />
          </div>
        )}
      </section>
    </ReorderActionsContext.Provider>
  )
}

/**
 * 메인 영역: 선택 바 + 갤러리/목록 + (페이지 모드) 쪽 이동 바 + 로딩·빈 상태·오류.
 * 탐색 방식은 설정(`galleryPaging`)을 따른다 — 설정이 오기 전에는 그리지 않아 모드가 바뀌며 두 번 받지 않는다.
 * 목차 레일(`IndexRail`)은 뷰포트 오른쪽에 붙고, `renderRail` 로 대체할 수 있다.
 */
export function Gallery({ renderRail }: { renderRail?: (controller: GalleryScrollController) => ReactNode }) {
  const { t } = useTranslation()
  const settings = useSettings()
  if (settings.isPending) {
    return (
      <section aria-label={t("gallery.section")} className="flex min-h-0 min-w-0 flex-1 flex-col">
        <LoadingSkeleton width={0} />
      </section>
    )
  }
  return (
    <GalleryBody
      paging={settings.data?.galleryPaging ?? "infinite"}
      pageSize={settings.data?.galleryPageSize ?? 100}
      indexRail={settings.data?.indexRail ?? "always"}
      titleEditEnabled={settings.data?.titleEditEnabled ?? false}
      capturedAtFallback={settings.data?.capturedAtFallback ?? DEFAULT_CAPTURED_AT_FALLBACK}
      renderRail={renderRail}
    />
  )
}
