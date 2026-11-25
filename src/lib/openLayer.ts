/** 열려 있는 대화상자·메뉴·셀렉트 — 열려 있으면 Esc 는 그쪽이 먼저 처리한다. 가려진 DOM 은 aria-hidden 이라 제외된다. */
export const OPEN_LAYER = '[role="dialog"], [role="alertdialog"], [role="menu"], [role="listbox"][data-state="open"], [data-radix-popper-content-wrapper]'

/** 입력 중인 요소(검색창·제목 편집 등)와 단축키 기록 중인 칸(`data-shortcut-capture`)에서는 Esc 를 그쪽이 처리한다 */
export function isEditableTarget(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && target.closest('input, textarea, select, [contenteditable="true"], [data-shortcut-capture]') !== null
}
