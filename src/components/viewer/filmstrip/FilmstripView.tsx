import { GalleryHorizontalEnd, ImageOff } from "lucide-react"
import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { EmptyState } from "@/components/gallery/EmptyState"
import { useElementSize } from "@/components/gallery/useElementWidth"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useAssetDetail } from "@/hooks/useAssetDetail"
import { cn } from "@/lib/utils"
import { usePlaybackStore, useSelectionStore, useViewStore, type GalleryStepper } from "@/stores"
import type { ClickMods } from "@/stores/selection"
import { ViewerBody, useViewerLifecycle } from "../ContentViewer"
import { ImageToolsSlotContext } from "../ImageToolsSlot"
import { PlaybackBar } from "../PlaybackBar"
import { PlaybackQueueList } from "../PlaybackQueueList"
import { useFullscreenIdle } from "../useFullscreenIdle"
import { useWindowFullscreen } from "../useWindowFullscreen"
import { FilmstripResizeHandle } from "./FilmstripResizeHandle"
import { FilmstripToolbar } from "./FilmstripToolbar"
import { FilmstripTrack, type FilmstripSource, type StripPageStep } from "./FilmstripTrack"
import { STRIP_PAD_Y, effectiveStripHeight } from "./filmstripMath"

export interface FilmstripViewProps {
  source: FilmstripSource
  positionOf: (assetId: number) => number | null
  /** 검색 조건 객체 — 바뀌면 이전 결과의 현재 항목을 버리고 새 결과의 첫 항목으로 */
  request: unknown
  /** 새 검색 결과를 기다리며 이전 결과를 보여 주는 중 — 새 결과가 도착해야 현재 항목을 다시 고른다 */
  refreshing: boolean
  selected: ReadonlySet<number>
  titleEditEnabled: boolean
  stepper: GalleryStepper
  onSelect: (id: number, mods: ClickMods) => void
  onJump: (edge: "first" | "last") => void
  onForwardKey?: (e: KeyboardEvent<HTMLElement>) => boolean
  pageStep?: StripPageStep
  /** 결과가 없거나 불러오는 중·오류일 때 갤러리가 보여 주는 상태 화면 — 있으면 보기·스트립 대신 그린다 */
  state?: ReactNode
}

interface BodyProps extends FilmstripViewProps {
  assetId: number
}

/** 툴바 + 무대(`ViewerBody` 그대로) + (연속 재생 바) + 높이 손잡이 + 스트립. 이동해도 이 트리는 유지된다 — 무대 안쪽만 항목별로 다시 마운트된다. */
function FilmstripBody({ assetId, source, positionOf, selected, titleEditEnabled, stepper, onSelect, onJump, onForwardKey, pageStep }: BodyProps) {
  const closePreview = useViewStore((s) => s.closePreview)
  const fullscreen = useViewStore((s) => s.fullscreen)
  const stripVisible = useViewStore((s) => s.stripVisible)
  const setStripVisible = useViewStore((s) => s.setStripVisible)
  const stored = useViewStore((s) => s.filmstripHeight)
  const playing = usePlaybackStore((s) => s.status !== "idle")
  const token = usePlaybackStore((s) => s.token)
  const { data } = useAssetDetail(assetId)
  const { t } = useTranslation()
  const idle = useFullscreenIdle(fullscreen)
  const [root, setRoot] = useState<HTMLElement | null>(null)
  const [slot, setSlot] = useState<HTMLDivElement | null>(null)
  const { height: rootH } = useElementSize(root)
  const stripH = effectiveStripHeight(stored, rootH)
  const pos = positionOf(assetId)
  const currentPos = pos !== null && pos >= source.range[0] && pos < source.range[1] ? pos : null

  return (
    <section
      ref={setRoot}
      aria-label={t("viewer.region")}
      data-idle={fullscreen && idle ? "true" : undefined}
      className={cn(
        "group/pane @container flex min-h-0 min-w-0 flex-1 flex-col bg-background",
        fullscreen && "fixed inset-0 z-40 bg-viewer-stage",
        fullscreen && idle && "**:cursor-none!"
      )}
    >
      {!fullscreen && (
        <FilmstripToolbar asset={data} assetId={assetId} stepper={stepper} stripVisible={stripVisible} onToggleStrip={() => setStripVisible(!stripVisible)} onClose={closePreview} setToolsSlot={setSlot} />
      )}
      <ImageToolsSlotContext.Provider value={fullscreen ? null : slot}>
        <div className="flex min-h-60 min-w-0 flex-1 flex-col">
          {/* 연속 재생 중에는 같은 항목이 반복돼도 새로 마운트되도록 token 을 키에 넣는다 */}
          <ViewerBody key={playing ? `${assetId}:${token}` : assetId} assetId={assetId} />
        </div>
      </ImageToolsSlotContext.Provider>
      {playing && !fullscreen && (
        <>
          <PlaybackBar />
          <PlaybackQueueList />
        </>
      )}
      {!fullscreen &&
        (stripVisible ? (
          <>
            <FilmstripResizeHandle />
            <div
              // 높이는 저장값을 창에 맞게 줄인 계산 값
              style={{ height: stripH }}
              className="flex shrink-0 flex-col py-1.5"
            >
              <FilmstripTrack
                source={source}
                currentId={assetId}
                currentPos={currentPos}
                tileHeight={stripH - STRIP_PAD_Y * 2}
                selected={selected}
                titleEditEnabled={titleEditEnabled}
                onSelect={onSelect}
                onStep={stepper.step}
                onJump={onJump}
                onForwardKey={onForwardKey}
                pageStep={pageStep}
              />
            </div>
          </>
        ) : (
          <Button variant="ghost" size="sm" className="h-6 shrink-0 rounded-none border-t border-border text-xs text-muted-foreground" onClick={() => setStripVisible(true)}>
            <GalleryHorizontalEnd aria-hidden="true" />
            {t("viewer.filmstrip.show")}
          </Button>
        ))}
    </section>
  )
}

/**
 * 필름스트립 보기 모드의 갤러리 자리 — 보기가 큰 무대를, 갤러리 조회 창이 아래 한 줄 스트립을 채운다. 분할 패널(`ContentViewer`)은 이 모드에서 그리지 않는다.
 * 현재 항목 = `previewId`(상세 패널은 활성 항목을 따르고, 활성 항목이 바뀌면 보기가 따라온다). 진입 시 활성 항목, 없으면 첫 결과.
 */
export function FilmstripView(props: FilmstripViewProps) {
  const { t } = useTranslation()
  const { source, positionOf, request, refreshing } = props
  const previewId = useViewStore((s) => s.previewId)
  useViewerLifecycle()
  useWindowFullscreen()

  // 진입(또는 검색이 바뀐 직후)에 보여 줄 항목 — 활성 항목이 결과에 있으면 그것, 없으면 첫 결과
  useEffect(() => {
    if (previewId !== null || source.total <= 0) return
    const active = useSelectionStore.getState().activeId
    if (active !== null && positionOf(active) !== null) {
      useViewStore.getState().openPreview(active)
      return
    }
    const first = source.getItem(source.range[0])
    if (first) {
      useSelectionStore.getState().selectOnly(first.assetId)
      useViewStore.getState().openPreview(first.assetId)
    }
  })
  // 검색 조건이 바뀌면 이전 결과의 항목을 버린다 — 새 결과가 도착한 뒤에(위 효과가 새 결과에서 항목을 고른다)
  const lastRequest = useRef(request)
  const resetPending = useRef(false)
  useEffect(() => {
    if (lastRequest.current !== request) {
      lastRequest.current = request
      resetPending.current = true
    }
    if (resetPending.current && !refreshing) {
      resetPending.current = false
      useViewStore.setState({ previewId: null, seekMs: null })
    }
  }, [request, refreshing])

  if (props.state || source.total <= 0) {
    return (
      <section aria-label={t("viewer.region")} className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
        {props.state ?? <EmptyState icon={ImageOff} title={t("viewer.filmstrip.emptyTitle")} description={t("viewer.filmstrip.emptyDescription")} />}
      </section>
    )
  }
  if (previewId === null) {
    return (
      <section aria-label={t("viewer.region")} className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
        <Skeleton role="status" aria-busy="true" aria-label={t("viewer.filmstrip.loading")} className="m-3 min-h-0 flex-1" />
      </section>
    )
  }
  return <FilmstripBody {...props} assetId={previewId} />
}
