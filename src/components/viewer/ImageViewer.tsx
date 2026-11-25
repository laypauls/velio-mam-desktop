import { Maximize, RotateCw, ZoomIn, ZoomOut } from "lucide-react"
import { useContext, useRef, useState, type PointerEvent, type WheelEvent } from "react"
import { createPortal } from "react-dom"
import { useTranslation } from "react-i18next"
import { useElementSize } from "@/components/gallery/useElementWidth"
import { Button } from "@/components/ui/button"
import { useApi } from "@/lib/api"
import type { AssetDetail } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { FullscreenButton } from "./FullscreenButton"
import { ImageToolsSlotContext } from "./ImageToolsSlot"
import { useFullscreenKey } from "./useFullscreenKey"
import { usePlaybackFailure } from "./usePlaybackFailure"
import { ViewerContextMenu } from "./ViewerContextMenu"
import { fitScale, WHEEL_STEP, zoomAround, ZOOM_STEP, type Size } from "./viewerMath"

interface View {
  /** true 면 영역에 맞춘 배율을 쓰고 이동량은 0 이다 */
  fit: boolean
  scale: number
  tx: number
  ty: number
  rotation: number
}

const FIT_VIEW: View = { fit: true, scale: 1, tx: 0, ty: 0, rotation: 0 }

/** 이미지 보기 — 맞춤/1:1/버튼·휠 확대(커서 기준)/드래그 이동/90° 회전(표시만). 원본 파일은 건드리지 않는다. */
export function ImageViewer({ asset, name, playable }: { asset: AssetDetail; name: string; playable: boolean }) {
  const { t } = useTranslation()
  const api = useApi()
  useFullscreenKey()
  const [stage, setStage] = useState<HTMLDivElement | null>(null)
  const box = useElementSize(stage)
  const [natural, setNatural] = useState<Size | null>(null)
  const [failed, setFailed] = useState(false)
  usePlaybackFailure(asset.assetId, failed)
  const [view, setView] = useState<View>(FIT_VIEW)
  const drag = useRef<{ x: number; y: number } | null>(null)
  const [dragging, setDragging] = useState(false)
  const slot = useContext(ImageToolsSlotContext)

  const fit = fitScale(box, natural, view.rotation)
  const scale = view.fit ? fit : view.scale
  // 재생 가능(웹 표시 가능) 형식은 원본, 아니면 1024 썸네일
  const src = playable ? api.mediaUrl(asset.assetId) : api.thumbUrl(asset.assetId, 1024)

  const zoomBy = (factor: number, px = 0, py = 0) =>
    setView((v) => {
      const base = v.fit ? { scale: fit, tx: 0, ty: 0 } : v
      return { ...v, fit: false, ...zoomAround(base, factor, px, py) }
    })
  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    const rect = e.currentTarget.getBoundingClientRect()
    zoomBy(e.deltaY < 0 ? WHEEL_STEP : 1 / WHEEL_STEP, e.clientX - rect.left - rect.width / 2, e.clientY - rect.top - rect.height / 2)
  }
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || view.fit) return
    drag.current = { x: e.clientX, y: e.clientY }
    setDragging(true)
    e.currentTarget.setPointerCapture?.(e.pointerId)
  }
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const from = drag.current
    if (!from) return
    drag.current = { x: e.clientX, y: e.clientY }
    const dx = e.clientX - from.x
    const dy = e.clientY - from.y
    setView((v) => ({ ...v, tx: v.tx + dx, ty: v.ty + dy }))
  }
  const endDrag = () => {
    drag.current = null
    setDragging(false)
  }

  const percent = Math.round(scale * 100)
  // 도구 줄 내용 — 떠 있는 줄과 툴바 슬롯(필름스트립)이 같은 버튼을 쓴다
  const tools = (
    <>
      <Button size="icon-sm" variant="ghost" aria-label={t("viewer.zoomOut")} onClick={() => zoomBy(1 / ZOOM_STEP)}>
        <ZoomOut aria-hidden="true" />
      </Button>
      <Button size="icon-sm" variant={view.fit ? "secondary" : "ghost"} aria-label={t("viewer.fit")} aria-pressed={view.fit} onClick={() => setView((v) => ({ ...FIT_VIEW, rotation: v.rotation }))}>
        <Maximize aria-hidden="true" />
      </Button>
      <Button
        size="icon-sm"
        variant={!view.fit && view.scale === 1 ? "secondary" : "ghost"}
        aria-label={t("viewer.actualSize")}
        aria-pressed={!view.fit && view.scale === 1}
        onClick={() => setView((v) => ({ ...v, fit: false, scale: 1, tx: 0, ty: 0 }))}
        className="text-xs tabular-nums"
      >
        1:1
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label={t("viewer.zoomIn")} onClick={() => zoomBy(ZOOM_STEP)}>
        <ZoomIn aria-hidden="true" />
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label={t("viewer.rotate")} onClick={() => setView((v) => ({ ...v, rotation: (v.rotation + 90) % 360 }))}>
        <RotateCw aria-hidden="true" />
      </Button>
      <span className="min-w-12 px-1 text-center text-xs text-muted-foreground tabular-nums" role="status" aria-label={t("viewer.zoomLevel", { percent })}>
        {percent}%
      </span>
    </>
  )
  return (
    <div className="relative flex min-h-0 flex-1 flex-col bg-viewer-stage">
      <ViewerContextMenu asset={asset}>
        <div
          ref={setStage}
          data-testid="image-stage"
          onWheel={onWheel}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          className={cn("relative flex min-h-0 flex-1 touch-none items-center justify-center overflow-hidden", !view.fit && (dragging ? "cursor-grabbing" : "cursor-grab"))}
        >
          {failed ? (
            <p role="alert" className="px-4 text-center text-sm break-keep text-overlay-foreground">
              {t("viewer.imageFailed")}
            </p>
          ) : (
            <img
              src={src}
              alt={name}
              draggable={false}
              data-scale={scale}
              data-rotation={view.rotation}
              onLoad={(e) => setNatural({ width: e.currentTarget.naturalWidth, height: e.currentTarget.naturalHeight })}
              onError={() => setFailed(true)}
              className={cn("max-h-none max-w-none shrink-0 select-none", natural === null && "invisible")}
              style={{ transform: `translate(${view.fit ? 0 : view.tx}px, ${view.fit ? 0 : view.ty}px) rotate(${view.rotation}deg) scale(${scale})` }}
            />
          )}
        </div>
      </ViewerContextMenu>
      {slot ? (
        createPortal(
          <div role="toolbar" aria-label={t("viewer.imageTools")} className="flex items-center gap-0.5">
            {tools}
          </div>,
          slot
        )
      ) : (
        <div
          role="toolbar"
          aria-label={t("viewer.imageTools")}
          className="absolute bottom-3 left-1/2 flex -translate-x-1/2 items-center gap-0.5 rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-md transition-[opacity,visibility] group-data-[idle=true]/pane:invisible group-data-[idle=true]/pane:opacity-0"
        >
          {tools}
          <FullscreenButton />
        </div>
      )}
    </div>
  )
}
