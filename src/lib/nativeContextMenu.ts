/** 문자를 입력하는 `<input>` 종류 — 체크박스·슬라이더·버튼 등은 편집 메뉴가 의미 없으므로 제외한다 */
const TEXT_INPUT_TYPES = new Set(["text", "search", "password", "number", "email", "url", "tel"])

/**
 * 브라우저 기본 편집 메뉴(잘라내기·붙여넣기·맞춤법)가 의미 있는 요소인가 — 텍스트 입력란·textarea·contenteditable.
 * `isEditableTarget`(openLayer.ts, Esc/단축키 양보용)과 다르다: 그쪽은 checkbox·select 등도 "입력 중"으로 본다.
 */
export function isTextEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  const field = target.closest("input, textarea")
  if (field instanceof HTMLTextAreaElement) return true
  if (field instanceof HTMLInputElement) return TEXT_INPUT_TYPES.has(field.type)
  const editable = target.closest("[contenteditable]")
  return editable !== null && editable.getAttribute("contenteditable") !== "false"
}

export interface NativeContextMenuOptions {
  /** 개발 빌드 여부 — 기본은 `import.meta.env.DEV`. DEV 에서만 Shift+우클릭으로 기본 메뉴(검사)를 허용한다 */
  isDev?: boolean
}

let current: (() => void) | null = null

/**
 * WebView2 기본 컨텍스트 메뉴(뒤로·새로 고침·저장·인쇄·검사 등)를 앱 전체에서 막는다.
 * **bubble 단계**여야 한다 — Radix `ContextMenuTrigger` 는 이미 `defaultPrevented` 이면 메뉴를 열지 않으므로 capture 에서 막으면 앱 메뉴가 깨진다.
 * 중복 설치(StrictMode·HMR)는 이전 리스너를 먼저 제거한다. 반환값은 제거 함수.
 */
export function installNativeContextMenuGuard(options: NativeContextMenuOptions = {}): () => void {
  current?.()
  const isDev = options.isDev ?? import.meta.env.DEV
  const onContextMenu = (e: MouseEvent) => {
    if (isTextEditableTarget(e.target)) return
    if (isDev && e.shiftKey) return
    e.preventDefault()
  }
  document.addEventListener("contextmenu", onContextMenu)
  const uninstall = () => {
    document.removeEventListener("contextmenu", onContextMenu)
    if (current === uninstall) current = null
  }
  current = uninstall
  return uninstall
}
