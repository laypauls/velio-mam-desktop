import { Fragment, useCallback, useRef } from "react"
import { useTranslation } from "react-i18next"
import { Skeleton } from "@/components/ui/skeleton"
import { sectionsInRange } from "@/hooks/galleryWindow/sections"
import { useSetFavorite, useSetRating, useToastError } from "@/hooks/useUserData"
import type { AssetSummary, CapturedAtFallback } from "@/lib/ipc-contract"
import { useViewStore } from "@/stores"
import { AssetCell, type CellHandlers } from "./AssetCell"
import { GAP, HEADER_H, type GalleryGeometry } from "./galleryGeometry"
import { monthSectionLabel } from "./grouping"

/** 보이는 영역 위아래로 더 그리는 높이(px) */
const OVERSCAN_PX = 240

interface Props extends CellHandlers {
  geo: GalleryGeometry
  offset: number
  getItem: (g: number) => AssetSummary | undefined
  selected: ReadonlySet<number>
  /** 키보드 포커스 항목 */
  activeId: number | null
  /** 큰 보기 직접 호출(카드 빠른 동작) — 생략하면 `onOpen` */
  onPreview?: (id: number) => void
  /** 제목 편집이 켜져 있으면 제목을 이름으로 표시 */
  titleEditEnabled?: boolean
  capturedAtFallback?: CapturedAtFallback
  /** 컬렉션 순서 정렬 — 카드에 `#n`(전역 위치 + 1)을 표시 */
  showOrder?: boolean
  /** 순서 재배치 드래그 가능 */
  draggable?: boolean
}

/**
 * 오프셋 기준 격자 — 섹션 모델에서 보이는 행만 그린다. 아직 받지 못한 칸은 스켈레톤.
 * 위치·크기는 가상화 계산값이라 동적 style 이 불가피하다(Tailwind 로 표현 불가).
 */
export function VirtualGrid({ geo, offset, getItem, selected, activeId, onSelect, onOpen, onPreview, titleEditEnabled, capturedAtFallback, showOrder = false, draggable = false }: Props) {
  const { t } = useTranslation()
  const display = useViewStore((s) => s.cardDisplay)
  const { model, cols, cell, cellH, rowH, padTop } = geo
  const selecting = selected.size > 0
  const { mutate: mutateFavorite } = useSetFavorite()
  const { mutate: mutateRating } = useSetRating()
  const toastError = useToastError()
  const onError = useRef(toastError)
  onError.current = toastError
  // 즐겨찾기만 바꾼다 — 카드 선택은 건드리지 않는다. 셀 memo 가 깨지지 않게 참조를 고정한다
  const onToggleFavorite = useCallback(
    (id: number, favorite: boolean) => mutateFavorite({ assetIds: [id], favorite }, { onError: (e) => onError.current(e) }),
    [mutateFavorite]
  )

  const onSetRating = useCallback(
    (id: number, rating: number) => mutateRating({ assetIds: [id], rating }, { onError: (e) => onError.current(e) }),
    [mutateRating]
  )

  if (geo.viewportH === 0 || cell === 0) return null
  const y0 = offset - padTop - OVERSCAN_PX
  const y1 = offset - padTop + geo.viewportH + OVERSCAN_PX
  const shift = padTop - offset // 모델 y → 화면 y

  return (
    <div role="presentation" className="relative h-full w-full overflow-hidden">
      {sectionsInRange(model, y0, y1).map((s) => {
        const r0 = Math.max(0, Math.floor((y0 - s.startY - s.headerH) / rowH))
        const r1 = Math.min(s.rows - 1, Math.floor((y1 - s.startY - s.headerH) / rowH))
        const label = s.headerH > 0 ? (monthSectionLabel(s.key) ?? t("gallery.noDate")) : null
        return (
          <Fragment key={`${s.sectionIndex}:${s.start}`}>
            {label !== null && (
              <div
                role="presentation"
                className="absolute top-0 left-0 w-full px-3"
                style={{ height: HEADER_H, transform: `translateY(${s.startY + shift}px)` }}
              >
                <div role="presentation" className="flex h-full items-end gap-3 pb-1 text-sm font-medium text-muted-foreground">
                  <span className="break-keep">{s.continued ? t("gallery.continued", { label }) : label}</span>
                  <span aria-hidden="true" className="mb-2 h-px flex-1 bg-border" />
                </div>
              </div>
            )}
            {Array.from({ length: Math.max(0, r1 - r0 + 1) }, (_, k) => {
              const row = r0 + k
              const first = s.start + row * cols
              const count = Math.min(cols, s.start + s.count - first)
              return (
                <div
                  key={first}
                  role="presentation"
                  className="absolute top-0 left-0 w-full px-3"
                  style={{ height: rowH, transform: `translateY(${s.startY + s.headerH + row * rowH + shift}px)` }}
                >
                  <div role="presentation" className="flex" style={{ gap: GAP }}>
                    {Array.from({ length: count }, (_, c) => {
                      const g = first + c
                      const asset = getItem(g)
                      return (
                        <div key={asset ? asset.assetId : `sk${g}`} role="presentation" className="shrink-0" style={{ width: cell, height: cellH }}>
                          {asset ? (
                            <AssetCell
                              asset={asset}
                              selected={selected.has(asset.assetId)}
                              active={asset.assetId === activeId}
                              display={display}
                              cellWidth={cell}
                              selecting={selecting}
                              onSelect={onSelect}
                              onOpen={onOpen}
                              onPreview={onPreview}
                              titleEditEnabled={titleEditEnabled}
                              capturedAtFallback={capturedAtFallback}
                              orderNumber={showOrder ? g + 1 : undefined}
                              draggable={draggable}
                              onToggleFavorite={onToggleFavorite}
                              onSetRating={onSetRating}
                            />
                          ) : (
                            <Skeleton data-placeholder="true" aria-hidden="true" className="size-full rounded-lg" />
                          )}
                        </div>
                      )
                    })}
                  </div>
                </div>
              )
            })}
          </Fragment>
        )
      })}
    </div>
  )
}
