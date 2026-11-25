import { vi } from "vitest"
import type { UiApi } from "@/lib/api"
import type { AssetDetail } from "@/lib/ipc-contract"
import { usePlaybackStore } from "@/stores"

/**
 * 재생 상태를 가진 가짜 미디어 요소 동작 — jsdom 은 play/pause/duration 을 구현하지 않는다.
 * `state.swallowFirstPlay`(기본 true): 요소마다 첫 `play()`(연속 재생 중이 아닐 때만) 는 "열 때 자동 재생 시도" 로 보고 호출 기록·재생 상태에 남기지 않는다 —
 * 영상은 정지 상태로 열리면 포스터가 먼저 뜨므로(VideoPoster) 열린 플레이어는 자동 재생으로만 만들어지지만, 기존 테스트는 "열린 직후 일시정지" 를 전제로 한다.
 * 자동 재생 자체를 검증하는 테스트는 열기 전에 false 로 둔다.
 */
export function stubMedia(duration = 100) {
  const state = { paused: true, swallowFirstPlay: true }
  const attempted = new WeakSet<HTMLMediaElement>()
  const fire = (el: Element, name: string) => el.dispatchEvent(new Event(name))
  vi.spyOn(HTMLMediaElement.prototype, "paused", "get").mockImplementation(() => state.paused)
  vi.spyOn(HTMLMediaElement.prototype, "duration", "get").mockReturnValue(duration)
  const play = vi.spyOn(HTMLMediaElement.prototype, "play").mockImplementation(function (this: HTMLMediaElement) {
    if (state.swallowFirstPlay && !attempted.has(this) && usePlaybackStore.getState().status === "idle") {
      attempted.add(this)
      play.mockClear() // 호출 기록에서도 뺀다 — 기존 테스트의 play() 횟수 단언은 사용자 조작만 센다
      return Promise.resolve()
    }
    state.paused = false
    fire(this, "play")
    return Promise.resolve()
  })
  const pause = vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(function (this: HTMLMediaElement) {
    state.paused = true
    fire(this, "pause")
  })
  return { play, pause, state }
}

/**
 * 모든 자산 상세를 주어진 형태(기본: 재생 가능한 mp4 영상)로 돌려주고, 내장 플레이어를 꺼서 `<video>`/`<audio>` 경로로 보내는 설정.
 * `renderApp` 전에 호출한다.
 */
export async function useStubbedDetail(api: UiApi, patch: Partial<AssetDetail> = {}): Promise<void> {
  const base = await api.getAssetDetail(1)
  const detail: AssetDetail = { ...base, mediaType: "video", playable: true, availability: "online", fileName: "clip.mp4", fps: 25, ...patch }
  vi.spyOn(api, "getAssetDetail").mockImplementation((id) => Promise.resolve({ ...detail, assetId: id }))
  vi.spyOn(api, "mediaUrl").mockImplementation((id) => `media://${id}`)
  // 열 때 자동 재생 — 정지 상태 설정이면 영상은 포스터가 먼저 뜬다 (VideoPoster.test.tsx)
  await api.updateSettings({ videoPlayerBuiltIn: false, mediaOpenAutoplay: true })
}
