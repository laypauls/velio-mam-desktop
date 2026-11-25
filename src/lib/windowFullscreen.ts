import { getCurrentWindow } from "@tauri-apps/api/window"

/** 전체화면 진입 전에 창이 최대화였는가 — 해제 뒤 같은 상태로 되돌린다 */
let wasMaximized = false
/** 진입·해제 호출을 한 줄로 세운다 — 빠른 토글이 겹쳐 창 상태가 뒤집히지 않게 */
let queue: Promise<void> = Promise.resolve()

async function apply(on: boolean): Promise<void> {
  const win = getCurrentWindow()
  if (on) {
    wasMaximized = await win.isMaximized()
    await win.setFullscreen(true)
    return
  }
  await win.setFullscreen(false)
  // 전체화면에서 돌아오면 최대화가 풀리는 경우가 있어 진입 전 상태로 복원한다
  if (wasMaximized && !(await win.isMaximized())) await win.maximize()
  wasMaximized = false
}

/** 메인 창 전체화면 진입/해제. 실패하면 reject — 호출자가 상태를 되돌린다 */
export function setWindowFullscreen(on: boolean): Promise<void> {
  const run = queue.then(() => apply(on))
  queue = run.catch(() => undefined)
  return run
}
