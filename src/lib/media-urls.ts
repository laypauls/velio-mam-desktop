// 커스텀 프로토콜 URL (P2-3 D5). 코어: crates/mam-core/src/protocol, 등록: src-tauri/src/protocols.rs.
//
// - `mamthumb`: 썸네일 320/1024 와 영상 프레임 스트립(`strip`) WebP. 없으면 404 (썸네일이 아직 없는 자산 - `thumbState` 참고).
// - `mammedia`: 원본 재생 (HTTP Range). 오프라인·파일 없음·비활성 자산은 404.
// WebView2(Windows) 는 `http://<scheme>.localhost/<경로>`, 그 밖에는 `<scheme>://localhost/<경로>` 이다.

export const THUMB_SCHEME = "mamthumb"
export const MEDIA_SCHEME = "mammedia"

/** 썸네일 긴 변(px). 계약 `ThumbSize` */
export type ThumbSize = 320 | 1024

/** 영상 마우스오버 미리보기: 같은 크기의 칸 5개를 가로로 이은 WebP 한 장. 칸 폭 = 이미지 폭 / 5 */
export const STRIP_FRAMES = 5

function base(scheme: string): string {
  const http =
    typeof navigator !== "undefined" && /windows|android/i.test(navigator.userAgent)
  return http ? `http://${scheme}.localhost` : `${scheme}://localhost`
}

function assertAssetId(assetId: number): void {
  if (!Number.isSafeInteger(assetId) || assetId <= 0) {
    throw new RangeError(`invalid asset id: ${assetId}`)
  }
}

/**
 * 썸네일 URL. `revision` 을 주면 `?v=` 로 붙여 자산이 바뀌었을 때 캐시를 우회한다 (서버는 쿼리를 무시한다).
 * 잘못된 id 는 예외 - 서버가 400 으로 거부할 요청을 만들지 않는다.
 */
export function thumbUrl(assetId: number, size: ThumbSize | "strip", revision?: number): string {
  assertAssetId(assetId)
  const v = revision === undefined ? "" : `?v=${revision}`
  return `${base(THUMB_SCHEME)}/${assetId}/${size}${v}`
}

/** 원본 재생 URL (`<video src>`/`<audio src>`/`<img src>`). */
export function mediaUrl(assetId: number): string {
  assertAssetId(assetId)
  return `${base(MEDIA_SCHEME)}/${assetId}`
}
