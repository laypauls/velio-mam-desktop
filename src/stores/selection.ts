import { create } from "zustand"

export interface ClickMods {
  ctrl: boolean
  shift: boolean
}

interface SelectionState {
  ids: ReadonlySet<number>
  /** Shift 범위 선택의 기준점 */
  anchor: number | null
  /** 상세 패널이 보여주는 자산 */
  activeId: number | null
  /** 전체 선택이 "불러온 항목"만 포함한다(결과 일부만 메모리에 있음) — 선택 바가 안내한다. 다른 선택 변경이 있으면 해제 */
  loadedOnly: boolean
  /** D9: 클릭=단일, Ctrl=토글, Shift=범위. `order` 는 현재 로드된 표시 순서. */
  click: (id: number, mods: ClickMods, order: readonly number[]) => void
  selectOnly: (id: number) => void
  selectAll: (order: readonly number[], opts?: { loadedOnly?: boolean }) => void
  clear: () => void
}

export const useSelectionStore = create<SelectionState>()((set, get) => ({
  ids: new Set(),
  anchor: null,
  activeId: null,
  loadedOnly: false,
  click: (id, { ctrl, shift }, order) => {
    const { ids, anchor } = get()
    if (shift && anchor !== null) {
      const a = order.indexOf(anchor)
      const b = order.indexOf(id)
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a]
        const range = order.slice(lo, hi + 1)
        set({ ids: new Set(ctrl ? [...ids, ...range] : range), activeId: id, loadedOnly: false })
        return
      }
    }
    if (ctrl) {
      const next = new Set(ids)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      set({ ids: next, anchor: id, activeId: id, loadedOnly: false })
      return
    }
    set({ ids: new Set([id]), anchor: id, activeId: id, loadedOnly: false })
  },
  selectOnly: (id) => set({ ids: new Set([id]), anchor: id, activeId: id, loadedOnly: false }),
  selectAll: (order, opts) => set({ ids: new Set(order), loadedOnly: opts?.loadedOnly === true }),
  clear: () => set({ ids: new Set(), anchor: null, loadedOnly: false }),
}))
