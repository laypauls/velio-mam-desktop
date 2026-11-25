import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react"
import { useSetRating, useToastError } from "@/hooks/useUserData"
import type { AssetSummary } from "@/lib/ipc-contract"
import { useSelectionStore, useViewStore, type GalleryStepper } from "@/stores"
import type { ClickMods } from "@/stores/selection"
import { isNavKey, nextIndex, type GalleryNav } from "./navigation"
import type { ReorderOp } from "./reorderTarget"
import type { ReorderActions } from "./useReorderActions"

/** Ctrl+Shift+키 → 순서 이동 (컬렉션 보기에서만) */
const REORDER_KEYS: Record<string, ReorderOp> = {
  ArrowUp: "up",
  ArrowLeft: "up",
  ArrowDown: "down",
  ArrowRight: "down",
  Home: "start",
  End: "end",
}

/** 페이지 모드의 쪽 끝 넘기 — 도착한 쪽에서 커서가 놓일 곳 */
export type PageLanding = "first" | "last"

export interface PageEdge {
  canStep: (dir: 1 | -1) => boolean
  /** 쪽을 옮긴다. 커서는 새 쪽이 준비되면 `landing` 항목에 놓인다 (Gallery 가 이동·위치 요청을 맡는다) */
  step: (dir: 1 | -1, landing: PageLanding) => void
}

interface Args {
  /** 이동 가능한 전역 범위 [start, end) — 무한 스크롤은 [0,total), 페이지 모드는 그 쪽 */
  range: readonly [number, number]
  nav: GalleryNav
  getItem: (g: number) => AssetSummary | undefined
  positionOf: (assetId: number) => number | null
  /** 메모리에 있는 항목의 id(위치 순) — Shift 범위 선택 */
  order: readonly number[]
  /** 목적지를 화면에 보이게 하고 그 블록을 요청한다 */
  reveal: (g: number) => void
  pageEdge?: PageEdge
  /** Enter 로 항목 열기 — 생략하면 큰 보기 (Gallery 는 더블클릭 설정을 따르는 열기를 넘긴다) */
  onOpen?: (id: number) => void
  /** 컬렉션 순서 이동 — 생략하면 Ctrl+Shift+화살표/Home/End 는 기존 동작 그대로 */
  reorder?: ReorderActions
}

type Move = "single" | "shift" | "ctrl"

/**
 * 갤러리 listbox 키보드 (aria-activedescendant 방식 — 포커스는 스크롤 컨테이너에 고정).
 * 커서는 전역 위치로 관리한다 — 아직 받지 못한 항목으로 이동하면 그 블록을 요청하고, 도착하면 포커스·선택을 적용한다.
 * Ctrl+화살표는 선택을 바꾸지 않고 cursor 만 움직인다.
 */
export function useGalleryKeyboard({ range, nav, getItem, positionOf, order, reveal, pageEdge, onOpen, reorder }: Args) {
  const [cursorPos, setCursorPos] = useState<number | null>(null)
  const pending = useRef<{ pos: number; how: Move } | null>(null)
  const cursorId = cursorPos !== null ? (getItem(cursorPos)?.assetId ?? null) : null
  const setRating = useSetRating()
  const toastError = useToastError()

  /** 로드된 항목이면 커서·선택을 적용한다. 아니면 false */
  const apply = (pos: number, how: Move): boolean => {
    const item = getItem(pos)
    if (!item) return false
    const sel = useSelectionStore.getState()
    const cursorItem = cursorPos !== null ? getItem(cursorPos) : undefined
    setCursorPos(pos)
    if (how === "ctrl") return true
    if (how === "shift") {
      // 기준점이 없으면 이동 전 항목을 기준점으로
      if (sel.anchor === null) sel.selectOnly(cursorItem?.assetId ?? item.assetId)
      useSelectionStore.getState().click(item.assetId, { ctrl: false, shift: true }, order)
    } else sel.selectOnly(item.assetId)
    return true
  }
  const applyRef = useRef(apply)
  applyRef.current = apply

  // 요청한 블록이 도착하면 보류했던 이동을 적용한다
  useEffect(() => {
    const p = pending.current
    if (p && getItem(p.pos)) {
      pending.current = null
      applyRef.current(p.pos, p.how)
    }
  })

  const moveTo = (pos: number, how: Move) => {
    reveal(pos)
    pending.current = null
    if (!apply(pos, how)) pending.current = { pos, how }
  }

  /** 마우스 선택도 cursor 를 따라가게 한다 */
  const onSelectWithCursor = useCallback(
    (id: number, mods: ClickMods) => {
      setCursorPos(positionOf(id))
      useSelectionStore.getState().click(id, mods, order)
    },
    [positionOf, order]
  )

  /** 쪽 이동으로 도착한 뒤 커서를 놓을 위치 (Gallery 가 새 쪽의 범위를 알게 된 뒤 호출) */
  const landOn = (pos: number) => {
    pending.current = { pos, how: "single" }
  }

  /** 전역 위치로 이동 (필름스트립 Home/End) — 아직 받지 못한 항목이면 블록을 요청하고 도착하면 적용한다 */
  const jumpTo = (pos: number) => moveTo(pos, "single")

  const reset = () => {
    pending.current = null
    setCursorPos(null)
  }

  const onKeyDown = (e: KeyboardEvent<HTMLElement>): boolean => {
    // 내부 컨트롤(체크박스 등)의 키 입력은 가로채지 않는다
    if (e.target !== e.currentTarget) return false
    const sel = useSelectionStore.getState()
    const ctrl = e.ctrlKey || e.metaKey
    const [start, end] = range
    const inRange = cursorPos !== null && cursorPos >= start && cursorPos < end
    const current = inRange ? cursorPos - start : -1

    // 1~5 = 별점 지정, 0 = 해제 — 선택이 있으면 선택 전체에, 없으면 커서 항목에 적용한다. 수정자 키와 함께면 다른 단축키에 양보한다
    // (입력 요소의 키 입력은 위의 target 검사로 이미 걸러진다)
    if (!ctrl && !e.altKey && !e.shiftKey && /^[0-5]$/.test(e.key)) {
      const ids = sel.ids.size > 0 ? [...sel.ids] : cursorId !== null ? [cursorId] : []
      if (ids.length === 0) return false
      e.preventDefault()
      setRating.mutate({ assetIds: ids, rating: Number(e.key) }, { onError: toastError })
      return true
    }
    if (e.key === "Enter") {
      // 대화상자로 포커스가 옮겨진 뒤 같은 Enter 가 새 포커스 버튼을 누르지 않게 한다
      e.preventDefault()
      const id = cursorId !== null ? cursorId : sel.activeId
      if (id !== null) (onOpen ?? ((i: number) => useViewStore.getState().openPreview(i)))(id)
      return true
    }
    if (e.key === " " && cursorId !== null) {
      e.preventDefault()
      sel.click(cursorId, { ctrl: true, shift: false }, order)
      return true
    }
    // 컬렉션 보기: Ctrl+Shift+↑/←/↓/→/Home/End = 순서 이동. 순서를 바꿀 수 없는 정렬이면 안내만 하고 키는 소비한다
    const reorderOp = reorder?.inCollection && ctrl && e.shiftKey && !e.altKey ? REORDER_KEYS[e.key] : undefined
    if (reorder && reorderOp) {
      e.preventDefault()
      if (!reorder.enabled) reorder.hint()
      else void reorder.run(reorderOp, sel.ids.size > 0 ? [...sel.ids] : cursorId !== null ? [cursorId] : [])
      return true
    }
    if (pageEdge && ctrl && (e.key === "PageDown" || e.key === "PageUp")) {
      const dir = e.key === "PageDown" ? 1 : -1
      e.preventDefault()
      if (pageEdge.canStep(dir)) pageEdge.step(dir, "first")
      return true
    }
    if (!isNavKey(e.key)) return false
    const next = nextIndex(e.key, current, end - start, nav)
    if (next === null) return false
    e.preventDefault()
    const dir = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0
    goTo(next, current, dir, ctrl ? "ctrl" : e.shiftKey ? "shift" : "single")
    return true
  }

  /** 범위 안 0-기준 위치 `next` 로 이동 — 쪽 끝에서 더 가려는 이동(`next === current`)은 이웃 쪽의 첫/마지막 항목으로 */
  const goTo = (next: number, current: number, dir: 1 | -1 | 0, how: Move) => {
    if (pageEdge && next === current && dir !== 0 && pageEdge.canStep(dir)) {
      pageEdge.step(dir, dir === 1 ? "first" : "last")
      return
    }
    moveTo(range[0] + next, how)
  }

  /**
   * 콘텐츠 보기 영역이 열어 둔 항목의 이전/다음 항목으로 활성 항목을 옮긴다 — 좌/우 화살표와 같은 경로(`goTo` → 선택 + 쪽 넘기기).
   * 보기 영역은 활성 항목을 따르므로 이동하면 새 항목이 열린다. 열린 항목이 이 목록(현재 쪽)에 없으면 아무것도 하지 않는다.
   */
  const stepViewer = (dir: 1 | -1) => {
    const id = useViewStore.getState().previewId
    const pos = id !== null ? positionOf(id) : null
    if (pos === null || pos < range[0] || pos >= range[1]) return
    const current = pos - range[0]
    const next = Math.min(range[1] - range[0] - 1, Math.max(0, current + dir))
    if (next === current && !pageEdge?.canStep(dir)) return
    goTo(next, current, dir, "single")
  }

  // 보기 영역 이동 버튼의 활성 상태 — 열린 항목이 이 목록 안에 있고 그 방향에 항목(또는 이웃 쪽)이 있을 때
  const previewId = useViewStore((s) => s.previewId)
  const viewerPos = previewId !== null ? positionOf(previewId) : null
  const listPos = viewerPos !== null && viewerPos >= range[0] && viewerPos < range[1] ? viewerPos : null
  const viewerStep: GalleryStepper = {
    canPrev: listPos !== null && (listPos > range[0] || pageEdge?.canStep(-1) === true),
    canNext: listPos !== null && (listPos < range[1] - 1 || pageEdge?.canStep(1) === true),
    step: stepViewer,
  }

  return { cursorPos, cursorId, reset, landOn, jumpTo, onSelectWithCursor, onKeyDown, viewerStep }
}
