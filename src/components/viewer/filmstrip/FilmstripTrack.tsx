import { ChevronLeft, ChevronRight } from "lucide-react"
import { useEffect, useMemo, useState, type KeyboardEvent, type MouseEvent, type WheelEvent } from "react"
import { useTranslation } from "react-i18next"
import { useElementWidth } from "@/components/gallery/useElementWidth"
import { Button } from "@/components/ui/button"
import type { AssetSummary } from "@/lib/ipc-contract"
import type { ClickMods } from "@/stores/selection"
import { FilmstripTile, tileId } from "./FilmstripTile"
import { stripWindow, tileWidth, TILE_GAP } from "./filmstripMath"

/** 스트립이 읽는 갤러리 조회 창 — 갤러리와 같은 블록 캐시를 쓰므로 전체 결과를 따로 올리지 않는다 */
export interface FilmstripSource {
  total: number
  /** 지금 보여 줄 전역 범위 [start, end) — 페이지 모드는 그 쪽, 무한 스크롤은 전체 */
  range: readonly [number, number]
  getItem: (pos: number) => AssetSummary | undefined
  /** 보이는 구간을 블록 캐시에 알린다 (무한 스크롤에서만 의미) */
  requestRange: (first: number, last: number) => void
}

/** 페이지 모드의 쪽 넘기기 — 스트립 양끝 버튼이 범위 끝에서 이웃 쪽으로 넘어갈 때 쓴다 */
export interface StripPageStep {
  canStep: (dir: 1 | -1) => boolean
  step: (dir: 1 | -1) => void
}

interface Props {
  source: FilmstripSource
  currentId: number | null
  /** 현재 항목의 전역 위치 — 아직 받지 못했거나 다른 쪽이면 null */
  currentPos: number | null
  /** 타일 높이(px) */
  tileHeight: number
  selected: ReadonlySet<number>
  titleEditEnabled: boolean
  onSelect: (id: number, mods: ClickMods) => void
  /** ←/→ — 현재 항목을 이전/다음으로 */
  onStep: (dir: 1 | -1) => void
  /** Home/End */
  onJump: (edge: "first" | "last") => void
  /** 숫자(별점)·스페이스 같은 갤러리 단축키를 그대로 넘긴다. 처리했으면 true */
  onForwardKey?: (e: KeyboardEvent<HTMLElement>) => boolean
  pageStep?: StripPageStep
}

/** 현재 항목을 둘러보다 벗어난 위치 — 현재 항목이 바뀌면(`forId`) 버린다 */
interface Browse {
  pos: number
  forId: number | null
}

const WHEEL_ITEMS = 2

/**
 * 가로 가상 스크롤 스트립 — 가운데 항목 기준으로 보이는 구간만 DOM 에 그린다. 현재 항목이 바뀌면 자동으로 가운데로 온다.
 * 좌/우 버튼·휠은 현재 항목을 바꾸지 않고 둘러보기만 한다(페이지 모드에서 범위 끝이면 이웃 쪽으로).
 * listbox 포커스는 스트립 요소에 두고 `aria-activedescendant` 로 현재 타일을 가리킨다.
 */
export function FilmstripTrack({ source, currentId, currentPos, tileHeight, selected, titleEditEnabled, onSelect, onStep, onJump, onForwardKey, pageStep }: Props) {
  const { t } = useTranslation()
  const { total, range, getItem, requestRange } = source
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null)
  const width = useElementWidth(viewport)
  const [browse, setBrowse] = useState<Browse | null>(null)

  const [r0, r1] = range
  const wanted = browse && browse.forId === currentId ? browse.pos : (currentPos ?? r0)
  const center = Math.min(Math.max(r0, r1 - 1), Math.max(r0, wanted))
  const widthOf = useMemo(() => (pos: number) => tileWidth(tileHeight, getItem(pos)), [tileHeight, getItem])
  const win = useMemo(() => stripWindow({ center, range, viewportW: width, widthOf }), [center, range, width, widthOf])

  const first = win?.first
  const last = win?.last
  useEffect(() => {
    if (first !== undefined && last !== undefined) requestRange(first, last)
  }, [first, last, requestRange])

  const visible = Math.max(1, win?.visible ?? 1)
  const browseBy = (dir: 1 | -1, count: number) => {
    const edge = dir > 0 ? r1 - 1 : r0
    if (center === edge && pageStep?.canStep(dir)) {
      pageStep.step(dir)
      setBrowse({ pos: dir > 0 ? r1 : r0 - 1, forId: currentId })
      return
    }
    setBrowse({ pos: Math.min(r1 - 1, Math.max(r0, center + dir * count)), forId: currentId })
  }
  const atStart = center <= r0 && !pageStep?.canStep(-1)
  const atEnd = center >= r1 - 1 && !pageStep?.canStep(1)

  const onClick = (e: MouseEvent<HTMLDivElement>, id: number) => {
    viewport?.focus({ preventScroll: true })
    onSelect(id, { ctrl: e.ctrlKey || e.metaKey, shift: e.shiftKey })
  }
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.ctrlKey || e.altKey || e.metaKey) return
    if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && !e.shiftKey) {
      e.preventDefault()
      onStep(e.key === "ArrowRight" ? 1 : -1)
    } else if ((e.key === "Home" || e.key === "End") && !e.shiftKey) {
      e.preventDefault()
      onJump(e.key === "Home" ? "first" : "last")
    } else if (/^[0-5]$/.test(e.key) || e.key === " ") onForwardKey?.(e)
  }
  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    if (e.deltaY !== 0) browseBy(e.deltaY > 0 ? 1 : -1, WHEEL_ITEMS)
  }

  const items: { pos: number; item: AssetSummary | undefined }[] = []
  if (win) for (let p = win.first; p <= win.last; p++) items.push({ pos: p, item: getItem(p) })

  return (
    <div className="flex min-h-0 flex-1 items-stretch gap-1 px-1">
      <Button size="icon-sm" variant="ghost" className="h-full w-6 shrink-0" aria-label={t("viewer.filmstrip.scrollLeft")} disabled={atStart} onClick={() => browseBy(-1, Math.round(visible * 0.8))}>
        <ChevronLeft aria-hidden="true" />
      </Button>
      <div
        ref={setViewport}
        role="listbox"
        tabIndex={0}
        aria-label={t("viewer.filmstrip.list")}
        aria-orientation="horizontal"
        aria-multiselectable="true"
        aria-activedescendant={currentId !== null && items.some((i) => i.item?.assetId === currentId) ? tileId(currentId) : undefined}
        onKeyDown={onKeyDown}
        onWheel={onWheel}
        className="relative min-w-0 flex-1 overflow-hidden rounded-md outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
      >
        <div
          // 위치는 가운데 기준 계산 값(offset) — 동적이라 인라인
          style={{ transform: `translateX(${win?.offset ?? 0}px)`, gap: TILE_GAP }}
          className="absolute inset-y-0 left-0 flex items-center"
        >
          {items.map(({ pos, item }) =>
            item ? (
              <FilmstripTile
                key={item.assetId}
                asset={item}
                pos={pos}
                total={total}
                width={tileWidth(tileHeight, item)}
                height={tileHeight}
                current={item.assetId === currentId}
                selected={selected.has(item.assetId)}
                titleEditEnabled={titleEditEnabled}
                onClick={onClick}
              />
            ) : (
              <div key={`ph-${pos}`} aria-hidden="true" data-placeholder="" className="shrink-0 rounded-md bg-muted" style={{ width: tileWidth(tileHeight, undefined), height: tileHeight }} />
            )
          )}
        </div>
      </div>
      <Button size="icon-sm" variant="ghost" className="h-full w-6 shrink-0" aria-label={t("viewer.filmstrip.scrollRight")} disabled={atEnd} onClick={() => browseBy(1, Math.round(visible * 0.8))}>
        <ChevronRight aria-hidden="true" />
      </Button>
    </div>
  )
}
