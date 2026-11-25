import { useEffect, useRef } from "react"
import { useTranslation } from "react-i18next"
import { Skeleton } from "@/components/ui/skeleton"
import { useAssetDetail } from "@/hooks/useAssetDetail"
import { useCloseViewerShortcut } from "@/hooks/useCloseViewerShortcut"
import { useDisplayName } from "@/hooks/useDisplayName"
import { useViewerPlacement } from "@/hooks/useViewerPlacement"
import { cn } from "@/lib/utils"
import { OPEN_LAYER, isEditableTarget } from "@/lib/openLayer"
import { usePlaybackStore, useSelectionStore, useViewStore } from "@/stores"
import { ImageViewer } from "./ImageViewer"
import { PlaybackBar } from "./PlaybackBar"
import { PlaybackQueueList } from "./PlaybackQueueList"
import { usePlaybackFailure } from "./usePlaybackFailure"
import { usePlaybackGuards } from "./usePlaybackGuards"
import { ViewerHeader } from "./ViewerHeader"
import { ViewerMedia } from "./ViewerMedia"
import { ViewerResizeHandle } from "./ViewerResizeHandle"
import { useFittedViewerWidth } from "./useFittedViewerWidth"
import { useFullscreenIdle } from "./useFullscreenIdle"
import { useWindowFullscreen } from "./useWindowFullscreen"

/** 열려 있는 동안만: Esc(전체화면이면 전체화면 해제만)·닫기 단축키(전체화면도 함께 해제)로 닫기, 활성 자산(마지막 클릭)을 따라가기 — 연속 재생 중에는 따라가지 않는다(카드 클릭 이동은 갤러리가 대기열 기준으로 처리) */
export function useViewerLifecycle() {
  const closePreview = useViewStore((s) => s.closePreview)
  useCloseViewerShortcut()
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented || isEditableTarget(e.target)) return
      if (document.querySelector(OPEN_LAYER)) return
      // 전체화면의 Esc 는 전체화면만 해제한다 — 보기 영역은 유지 (닫기는 Q·닫기 버튼)
      if (useViewStore.getState().fullscreen) {
        e.preventDefault()
        useViewStore.getState().setFullscreen(false)
        return
      }
      closePreview()
    }
    window.addEventListener("keydown", onKeyDown)
    return () => window.removeEventListener("keydown", onKeyDown)
  }, [closePreview])
  useEffect(
    () =>
      useSelectionStore.subscribe((s, prev) => {
        if (usePlaybackStore.getState().status !== "idle") return
        if (s.activeId !== null && s.activeId !== prev.activeId) useViewStore.getState().openPreview(s.activeId)
      }),
    []
  )
}

export function ViewerBody({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const { nameOf } = useDisplayName()
  const { data, isPending, isError } = useAssetDetail(assetId)
  // 상세를 못 불러오거나 저장 장치가 끊긴 항목은 연속 재생이 건너뛴다
  usePlaybackFailure(assetId, isError || (data !== undefined && data.availability !== "online"))

  if (isPending) {
    return <Skeleton role="status" aria-busy="true" aria-label={t("common.loading")} className="m-3 min-h-0 flex-1" />
  }
  if (isError || !data) {
    return (
      <p role="alert" className="p-3 text-sm text-destructive">
        {t("detail.error")}
      </p>
    )
  }
  const offline = data.availability !== "online"
  const playable = data.playable && !offline
  const name = nameOf(data)
  if (data.mediaType === "image") return <ImageViewer key={data.assetId} asset={data} name={name} playable={playable} />
  if (offline) {
    return (
      <div role="status" className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 bg-viewer-stage p-6 text-center text-sm text-overlay-foreground">
        <p className="break-keep">{t("preview.unavailable")}</p>
      </div>
    )
  }
  // 영상·음성 재생 경로(설정 "동영상을 내장 플레이어로 재생" 반영)는 ViewerMedia 가 결정한다
  return <ViewerMedia key={data.assetId} asset={data} name={name} playable={playable} />
}

function ViewerPane({ assetId }: { assetId: number }) {
  const { t } = useTranslation()
  const close = useViewStore((s) => s.closePreview)
  const paneRef = useRef<HTMLElement>(null)
  const playing = usePlaybackStore((s) => s.status !== "idle")
  const token = usePlaybackStore((s) => s.token)
  const { data } = useAssetDetail(assetId)
  useViewerLifecycle()
  useWindowFullscreen()
  const fullscreen = useViewStore((s) => s.fullscreen)
  const idle = useFullscreenIdle(fullscreen)
  const left = useViewerPlacement() === "left"
  // 저장된 폭을 현재 창에 맞게 줄인 표시 폭 — 저장 값은 그대로
  const width = useFittedViewerWidth(paneRef, left ? "left" : "right")
  // 전체화면에서는 분할선을 숨긴다 (자리 표시 false 라 aside 의 트리 위치는 그대로 — 플레이어가 다시 마운트되지 않는다)
  const handle = fullscreen ? null : <ViewerResizeHandle paneRef={paneRef} side={left ? "left" : "right"} />
  return (
    <>
      {!left && handle}
      <aside
        ref={paneRef}
        aria-label={t("viewer.region")}
        // 사용자가 끌어 정한 폭을 창에 맞게 줄인 계산 값 — 최소는 min-w
        style={{ flexBasis: width }}
        // 전체화면은 같은 요소를 창 전체로 키운다 (재마운트 없음). 마우스가 멈추면(data-idle) 컨트롤과 커서를 숨긴다
        data-idle={fullscreen && idle ? "true" : undefined}
        className={cn(
          "group/pane flex min-w-80 shrink flex-col border-border bg-background",
          left ? "border-r" : "border-l",
          fullscreen && "fixed inset-0 z-40 border-0 bg-viewer-stage",
          fullscreen && idle && "**:cursor-none!"
        )}
      >
        {!fullscreen && <ViewerHeader asset={data} onClose={close} />}
        {/* 연속 재생 중에는 같은 항목이 반복돼도 새로 마운트되도록 token 을 키에 넣는다 */}
        <ViewerBody key={playing ? `${assetId}:${token}` : assetId} assetId={assetId} />
        {playing && !fullscreen && (
          <>
            <PlaybackBar />
            <PlaybackQueueList />
          </>
        )}
      </aside>
      {left && handle}
    </>
  )
}

/** 분할 보기 영역 — 갤러리 오른쪽(설정에 따라 창 맨 왼쪽)의 콘텐츠 보기 영역 — `previewId` 가 있을 때만 나타난다 (이미지 보기 / 영상·음성 플레이어 / 안내). */
export function ContentViewer() {
  usePlaybackGuards()
  const previewId = useViewStore((s) => s.previewId)
  // 필름스트립 모드에서는 보기가 갤러리 자리(FilmstripView)에 그려진다 — 분할 패널은 없다
  const filmstrip = useViewStore((s) => s.viewMode === "filmstrip")
  return previewId === null || filmstrip ? null : <ViewerPane assetId={previewId} />
}
