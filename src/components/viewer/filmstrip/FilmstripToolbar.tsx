import { ChevronLeft, ChevronRight, GalleryHorizontalEnd, Play, X } from "lucide-react"
import { useTranslation } from "react-i18next"
import { MediaTypeIcon } from "@/components/gallery/AssetThumb"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Skeleton } from "@/components/ui/skeleton"
import { useDisplayName } from "@/hooks/useDisplayName"
import { useStartPlayback } from "@/hooks/usePlayQueue"
import type { AssetDetail } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { usePlaybackStore, useSearchStore, type GalleryStepper } from "@/stores"
import { FullscreenButton } from "../FullscreenButton"
import { TYPE_COLOR, viewerMeta } from "../ViewerHeader"

interface Props {
  asset: AssetDetail | undefined
  assetId: number
  stepper: GalleryStepper
  stripVisible: boolean
  onToggleStrip: () => void
  onClose: () => void
  /** 이미지 도구(`ImageViewer`)가 포털로 들어올 자리 */
  setToolsSlot: (el: HTMLDivElement | null) => void
}

/**
 * 필름스트립 보기 툴바 — 왼쪽 유형 배지·이름, 가운데 이전/다음·연속 재생·이미지 도구(슬롯), 오른쪽 스트립 토글·전체화면·닫기.
 * 즐겨찾기·별점·태그 버튼은 상세 패널과 중복이라 두지 않는다.
 */
export function FilmstripToolbar({ asset, assetId, stepper, stripVisible, onToggleStrip, onClose, setToolsSlot }: Props) {
  const { t } = useTranslation()
  const { nameOf, tooltipOf } = useDisplayName()
  const view = useSearchStore((s) => s.view)
  const playing = usePlaybackStore((s) => s.status !== "idle")
  const start = useStartPlayback()
  const meta = asset ? viewerMeta(asset) : ""
  const stripLabel = t(stripVisible ? "viewer.filmstrip.hide" : "viewer.filmstrip.show")
  return (
    <div role="toolbar" aria-label={t("viewer.filmstrip.tools")} className="relative flex h-10 shrink-0 items-center justify-center gap-0.5 border-b border-border px-2">
      <div className="absolute left-2 flex max-w-[28%] items-center gap-2">
        {asset ? (
          <Badge variant="outline" className={cn("shrink-0 rounded-sm", TYPE_COLOR[asset.mediaType])}>
            <MediaTypeIcon type={asset.mediaType} />
            {t(`asset.type.${asset.mediaType}`)}
          </Badge>
        ) : (
          <Skeleton className="h-5 w-14" />
        )}
        <h2 className="min-w-0 truncate text-sm font-medium" title={asset ? tooltipOf(asset) : undefined}>
          {asset ? nameOf(asset) : ""}
        </h2>
        {meta !== "" && <span className="hidden shrink-0 text-xs text-muted-foreground tabular-nums @4xl:inline">{meta}</span>}
      </div>
      <Button size="icon-sm" variant="ghost" aria-label={t("viewer.contentPrev")} title={`${t("viewer.contentPrev")} [←]`} disabled={!stepper.canPrev} onClick={() => stepper.step(-1)}>
        <ChevronLeft aria-hidden="true" />
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label={t("viewer.contentNext")} title={`${t("viewer.contentNext")} [→]`} disabled={!stepper.canNext} onClick={() => stepper.step(1)}>
        <ChevronRight aria-hidden="true" />
      </Button>
      {/* 연속 재생은 컬렉션 보기에서만 시작할 수 있다 (기존 규칙) — 재생 중에는 아래 재생 바가 맡는다 */}
      {view.kind === "collection" && !playing && (
        <Button size="icon-sm" variant="ghost" aria-label={t("viewer.filmstrip.play")} title={t("viewer.filmstrip.play")} onClick={() => void start(view.collectionId, assetId)}>
          <Play aria-hidden="true" />
        </Button>
      )}
      <div ref={setToolsSlot} className="ml-2 flex items-center" />
      <div className="absolute right-1 flex items-center gap-0.5">
        <Button size="icon-sm" variant={stripVisible ? "secondary" : "ghost"} aria-label={stripLabel} title={stripLabel} aria-pressed={stripVisible} onClick={onToggleStrip}>
          <GalleryHorizontalEnd aria-hidden="true" />
        </Button>
        <FullscreenButton />
        <Button size="icon-sm" variant="ghost" aria-label={t("viewer.close")} title={`${t("viewer.close")} [Esc]`} onClick={onClose}>
          <X aria-hidden="true" />
        </Button>
      </div>
    </div>
  )
}
