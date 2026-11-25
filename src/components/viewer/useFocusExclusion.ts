import { useEffect, type RefObject } from "react"

/** 영역 안에서 포커스를 받을 수 있는 요소 — 전부 Tab 순서에서 뺀다 */
const FOCUSABLE = 'button, a[href], input, select, textarea, [role="slider"], [role="combobox"], [tabindex]'
/** 이 안(포털 목록·대화상자)으로 옮겨간 포커스는 "원래 포커스 위치"로 기억하지 않는다 */
const TRANSIENT_LAYER = '[role="listbox"], [role="menu"], [role="dialog"], [role="alertdialog"]'

function excludeFromTabOrder(root: HTMLElement) {
  for (const el of root.querySelectorAll<HTMLElement>(FOCUSABLE)) {
    if (el.getAttribute("tabindex") !== "-1") el.setAttribute("tabindex", "-1")
  }
}

/**
 * 플레이어 영역(컨트롤 바·구간 레인·연속 재생 바·대기열)을 포커스에서 완전히 제외한다 — 키보드 Tab 으로 닿지 않고, 마우스로 눌러도(드래그 포함) 포커스가 옮겨가지 않는다.
 * 그래서 Space 등 플레이어 단축키가 방금 누른 컨트롤에 가로채이지 않는다. 마우스 조작은 모두 그대로 동작한다.
 *
 * - Tab 제외: 영역 안의 포커스 가능한 요소에 `tabindex=-1` (Radix 슬라이더 손잡이·셀렉트 트리거 포함, 나중에 그려지는 요소는 MutationObserver 로)
 * - 누를 때 포커스 유지: mousedown 기본 동작(포커스 이동)을 막고, Radix Slider 처럼 pointerdown 에서 `focus()` 를 직접 부르거나 셀렉트가 닫히며 트리거를 포커스하는 경우에는
 *   영역 안으로 들어온 포커스를 누르기 전 위치로 되돌린다(없었으면 blur).
 *
 * 영역 밖(보기 영역 헤더 등)은 영향받지 않는다.
 */
export function useFocusExclusion(ref: RefObject<HTMLElement | null>) {
  useEffect(() => {
    const root = ref.current
    if (!root) return
    const active = document.activeElement
    let outside: HTMLElement | null = active instanceof HTMLElement && active !== document.body && !root.contains(active) ? active : null

    excludeFromTabOrder(root)
    const observer = new MutationObserver(() => excludeFromTabOrder(root))
    observer.observe(root, { childList: true, subtree: true, attributes: true, attributeFilter: ["tabindex"] })

    const onMouseDown = (e: MouseEvent) => e.preventDefault()
    const onFocusIn = (e: FocusEvent) => {
      const target = e.target instanceof HTMLElement ? e.target : null
      if (!target) return
      if (!root.contains(target)) {
        if (!target.closest(TRANSIENT_LAYER) && target !== document.body) outside = target
        return
      }
      if (outside && outside.isConnected) outside.focus({ preventScroll: true })
      else target.blur()
    }
    // 영역 밖 포커스가 아무 요소도 아닌 곳(빈 곳 클릭 등)으로 빠지면 "원래 위치 = 없음" — 낡은 요소로 되돌리지 않는다
    const onFocusOut = (e: FocusEvent) => {
      if (e.relatedTarget === null && e.target instanceof Node && !root.contains(e.target)) outside = null
    }
    root.addEventListener("mousedown", onMouseDown)
    document.addEventListener("focusin", onFocusIn)
    document.addEventListener("focusout", onFocusOut)
    return () => {
      observer.disconnect()
      root.removeEventListener("mousedown", onMouseDown)
      document.removeEventListener("focusin", onFocusIn)
      document.removeEventListener("focusout", onFocusOut)
    }
  }, [ref])
}
