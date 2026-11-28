import { getCurrentWindow } from "@tauri-apps/api/window"

/**
 * 사용자 정의 제목 표시줄의 창 조작 래퍼 — Tauri 창이 없는 환경(브라우저·테스트)에서는 조용히 무시한다.
 * 닫기는 `close()` 다: `src-tauri/src/tray.rs` 의 `CloseRequested` 처리("트레이로 닫기" 설정 ON + 설치본이면 숨김)를 그대로 탄다. `destroy()` 는 그 경로를 건너뛰므로 쓰지 않는다.
 */
function run(action: (win: ReturnType<typeof getCurrentWindow>) => Promise<unknown>): Promise<void> {
  try {
    return action(getCurrentWindow()).then(
      () => undefined,
      () => undefined
    )
  } catch {
    return Promise.resolve()
  }
}

export const minimizeWindow = (): Promise<void> => run((w) => w.minimize())
export const toggleMaximizeWindow = (): Promise<void> => run((w) => w.toggleMaximize())
export const closeWindow = (): Promise<void> => run((w) => w.close())

/**
 * 창 최대화 상태를 구독한다 — 시작 시 한 번 읽고, 크기 변경 이벤트(Win+↑·스냅·더블클릭·버튼 모두)마다 다시 읽는다.
 * 반환값은 구독 해제 함수. Tauri 창이 없으면 아무것도 하지 않는다.
 */
export function watchWindowMaximized(onChange: (maximized: boolean) => void): () => void {
  let cancelled = false
  let unlisten: (() => void) | undefined
  try {
    const win = getCurrentWindow()
    const sync = () => {
      win.isMaximized().then(
        (maximized) => {
          if (!cancelled) onChange(maximized)
        },
        () => undefined
      )
    }
    sync()
    win.onResized(sync).then(
      (off) => {
        if (cancelled) off()
        else unlisten = off
      },
      () => undefined
    )
  } catch {
    // Tauri 창이 없는 환경
  }
  return () => {
    cancelled = true
    unlisten?.()
  }
}
