import { MEDIA_SCHEME, THUMB_SCHEME, type ThumbSize } from "@/lib/ipc-contract"
import type { MediaUrls } from "./types"

/** Windows WebView2 는 커스텀 프로토콜을 `http://<scheme>.localhost/` 로 노출한다 (그 외는 `<scheme>://localhost/`). */
export function createProtocolUrls(isWindows: boolean): MediaUrls {
  const base = (scheme: string) => (isWindows ? `http://${scheme}.localhost` : `${scheme}://localhost`)
  return {
    thumbUrl: (assetId: number, size: ThumbSize) => `${base(THUMB_SCHEME)}/${assetId}/${size}`,
    mediaUrl: (assetId: number) => `${base(MEDIA_SCHEME)}/${assetId}`,
  }
}

export function detectWindows(): boolean {
  return typeof navigator !== "undefined" && /windows/i.test(navigator.userAgent)
}
