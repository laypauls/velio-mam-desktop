import { vi } from "vitest"
import type { ChromeWebview, SharedBufferEvent, WebMessageEvent } from "@/lib/chromeWebview"
import { PLAYER_WIRE, type BufferInfo, type FrameMsg, type ReleaseMsg } from "@/lib/ipc-player-wire"

/** 와이어 골든 예시와 같은 모양의 BufferInfo (기본 1280×720, stride 5120) */
export function makeBufferInfo(over: Partial<BufferInfo> = {}): BufferInfo {
  const w = over.w ?? 1280
  const h = over.h ?? 720
  const stride = over.stride ?? Math.ceil((w * 4) / 64) * 64
  return { mam: PLAYER_WIRE, epoch: 1, slot: 0, count: 3, size: stride * h, w, h, stride, fmt: "bgr0", ...over }
}

/** 호스트가 보내는 프레임 메시지 (기본 에포크 1, 슬롯 0, seq 1) */
export function makeFrame(over: Partial<FrameMsg> = {}): FrameMsg {
  const w = over.w ?? 1280
  const h = over.h ?? 720
  return { mam: PLAYER_WIRE, k: "frame", epoch: 1, slot: 0, seq: 1, w, h, stride: Math.ceil((w * 4) / 64) * 64, renderDoneMs: 1000, ...over }
}

export interface FakeWebView2 {
  webview: ChromeWebview
  /** 호스트로 보낸 release 들 (보낸 순서) — `mountMpv` 가 `api.playerReleaseSlots` 호출을 `recordRelease` 로 모은다 */
  posted: unknown[]
  recordRelease: (msg: unknown) => void
  /** `chrome.webview.postMessage` 호출 — 플레이어는 절대 부르지 않아야 한다 (Tauri 가 객체 메시지를 거부해 호스트에 닿지 않는다) */
  chromePostMessage: ReturnType<typeof vi.fn>
  /** 보낸 release 메시지만 */
  releases: () => ReleaseMsg[]
  /** `releaseBuffer` 로 놓은 버퍼 */
  freed: ArrayBuffer[]
  /** `window.ipc.postMessage`(Tauri IPC 폴백) 호출 — 플레이어는 절대 부르지 않아야 한다 */
  ipcPostMessage: ReturnType<typeof vi.fn>
  listenerCount: (type: "sharedbufferreceived" | "message") => number
  /** `sharedbufferreceived` 를 보낸다 — 만들어 준 ArrayBuffer 를 돌려준다 (`byteLength` 기본 info.size) */
  emitBuffer: (info: unknown, byteLength?: number) => ArrayBuffer
  /** 한 에포크의 세 슬롯 버퍼를 모두 보낸다 */
  emitBank: (epoch: number, over?: Partial<BufferInfo>) => ArrayBuffer[]
  /** `message`(호스트 → JS JSON) 를 보낸다 */
  emitFrame: (msg: unknown) => void
}

/**
 * `window.chrome.webview` 스텁 — 리스너·releaseBuffer 를 기록하고 `postMessage` 는 호출되면 안 되는 스파이로 건다. `window.ipc.postMessage` 도 스파이로 건다.
 * `vi.unstubAllGlobals()` 로 되돌린다. 결정적이어야 하므로 타이머·rAF 는 건드리지 않는다.
 */
export function installFakeWebView2(): FakeWebView2 {
  const target = new EventTarget()
  const posted: unknown[] = []
  const freed: ArrayBuffer[] = []
  const counts = { sharedbufferreceived: 0, message: 0 }
  const listeners = new Map<unknown, EventListener>()
  const chromePostMessage = vi.fn()
  const webview: ChromeWebview & { postMessage: typeof chromePostMessage } = {
    postMessage: chromePostMessage,
    releaseBuffer: (b: ArrayBuffer) => void freed.push(b),
    addEventListener: ((type: "sharedbufferreceived" | "message", l: (e: never) => void) => {
      const wrapped = l as unknown as EventListener
      listeners.set(l, wrapped)
      counts[type] += 1
      target.addEventListener(type, wrapped)
    }) as ChromeWebview["addEventListener"],
    removeEventListener: ((type: "sharedbufferreceived" | "message", l: (e: never) => void) => {
      const wrapped = listeners.get(l)
      if (!wrapped) return
      listeners.delete(l)
      counts[type] -= 1
      target.removeEventListener(type, wrapped)
    }) as ChromeWebview["removeEventListener"],
  }
  const ipcPostMessage = vi.fn()
  vi.stubGlobal("chrome", { webview })
  vi.stubGlobal("ipc", { postMessage: ipcPostMessage })
  const emitBuffer = (info: unknown, byteLength?: number) => {
    const size = typeof info === "object" && info !== null && "size" in info && typeof info.size === "number" ? info.size : 0
    const buffer = new ArrayBuffer(byteLength ?? size)
    const event: SharedBufferEvent = Object.assign(new Event("sharedbufferreceived"), { additionalData: info, getBuffer: () => buffer })
    target.dispatchEvent(event)
    return buffer
  }
  return {
    webview,
    posted,
    recordRelease: (msg) => void posted.push(msg),
    chromePostMessage,
    releases: () => posted.filter((m): m is ReleaseMsg => typeof m === "object" && m !== null && (m as { k?: unknown }).k === "release"),
    freed,
    ipcPostMessage,
    listenerCount: (type) => counts[type],
    emitBuffer,
    emitBank: (epoch, over = {}) => [0, 1, 2].map((slot) => emitBuffer(makeBufferInfo({ ...over, epoch, slot }))),
    emitFrame: (msg) => {
      const event: WebMessageEvent = Object.assign(new Event("message"), { data: msg })
      target.dispatchEvent(event)
    },
  }
}
