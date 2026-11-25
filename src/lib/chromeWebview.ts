/**
 * `window.chrome.webview` (WebView2) 의 타입이 있는 접근자 — 테스트가 `window.chrome` 을 스텁할 수 있도록 매번 읽는다.
 * 플레이어 프레임 **수신** 전용이다 (SharedBuffer·메시지 리스너, `releaseBuffer`). 호스트로 보내는 release 는 이 객체의
 * `postMessage` 가 아니라 Tauri command(`player_release_slots`)로 보낸다 — wry 의 메시지 처리기가 객체 메시지에서 오류를 내
 * WebView2 가 처리기 체인을 멈추므로 호스트에 닿지 않는다.
 */

/** `sharedbufferreceived` — 호스트가 `PostSharedBufferToScript` 로 보낸 버퍼 */
export interface SharedBufferEvent extends Event {
  additionalData?: unknown
  getBuffer(): ArrayBuffer
}

/** `message` — 호스트가 `PostWebMessageAsJson` 으로 보낸 JSON (`data` 는 파싱된 값) */
export interface WebMessageEvent extends Event {
  data?: unknown
}

export interface ChromeWebview {
  /** 받은 SharedBuffer 의 매핑을 놓는다. 호출 뒤에는 그 ArrayBuffer(와 파생 TypedArray)를 만지지 않는다 */
  releaseBuffer(buffer: ArrayBuffer): void
  addEventListener(type: "sharedbufferreceived", listener: (e: SharedBufferEvent) => void): void
  addEventListener(type: "message", listener: (e: WebMessageEvent) => void): void
  removeEventListener(type: "sharedbufferreceived", listener: (e: SharedBufferEvent) => void): void
  removeEventListener(type: "message", listener: (e: WebMessageEvent) => void): void
}

interface ChromeHost {
  chrome?: { webview?: ChromeWebview }
}

/** WebView2 가 아니거나 SharedBuffer 를 지원하지 않는 환경(브라우저·jsdom)이면 null */
export function getChromeWebview(): ChromeWebview | null {
  const webview = (window as unknown as ChromeHost).chrome?.webview
  if (!webview || typeof webview.releaseBuffer !== "function") return null
  return webview
}
