import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { expect, vi } from "vitest"
import { mergeRanges } from "@/components/viewer/playRanges"
import { ContentViewer } from "@/components/viewer/ContentViewer"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi, type MockOptions } from "@/lib/api/mock"
import type { AssetDetail, PlayQueueItem, PlayRange, SettingsPatch } from "@/lib/ipc-contract"
import { qk } from "@/hooks/keys"
import { resetAllStores, useViewStore } from "@/stores"
import { stubLayout } from "./layout"
import { stubMedia, useStubbedDetail } from "./media"

/**
 * requestVideoFrameCallback / requestAnimationFrame 를 결정적으로 스텁한다 (콜백·취소 API 를 함께) —
 * 콜백은 `frame`/`tick` 으로만 실행된다. stubLayout 의 rAF 스텁을 덮어쓰므로 그 뒤에 호출한다.
 */
export function stubFrames() {
  let next = 1
  const video = new Map<number, (now: number, meta: { mediaTime: number }) => void>()
  const raf = new Map<number, FrameRequestCallback>()
  const proto = HTMLVideoElement.prototype as unknown as {
    requestVideoFrameCallback?: unknown
    cancelVideoFrameCallback?: unknown
  }
  proto.requestVideoFrameCallback = (cb: (now: number, meta: { mediaTime: number }) => void) => {
    video.set(next, cb)
    return next++
  }
  proto.cancelVideoFrameCallback = (id: number) => void video.delete(id)
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    raf.set(next, cb)
    return next++
  })
  vi.stubGlobal("cancelAnimationFrame", (id: number) => void raf.delete(id))
  return {
    /** 대기 중인 영상 프레임 콜백 수 */
    pendingVideo: () => video.size,
    /** 대기 중인 rAF 콜백 수 (다른 소비자 포함) */
    pendingRaf: () => raf.size,
    /** 영상 프레임 콜백 실행 — mediaTime(초) */
    frame: (mediaTimeSec: number) => {
      const cbs = [...video.values()]
      video.clear()
      for (const cb of cbs) cb(0, { mediaTime: mediaTimeSec })
    },
    /** rAF 콜백 실행 (음성 / rVFC 없는 환경) */
    tick: () => {
      const cbs = [...raf.values()]
      raf.clear()
      for (const cb of cbs) cb(0)
    },
  }
}

/** stubFrames 가 prototype 에 붙인 rVFC 를 되돌린다 (afterEach) */
export function restoreFrames() {
  const proto = HTMLVideoElement.prototype as unknown as { requestVideoFrameCallback?: unknown; cancelVideoFrameCallback?: unknown }
  delete proto.requestVideoFrameCallback
  delete proto.cancelVideoFrameCallback
}

/** 연속 재생 대기열 항목 (영상) */
export const queueItem = (assetId: number): PlayQueueItem => ({
  assetId,
  mediaType: "video",
  availability: "online",
  fileName: `v${assetId}.mxf`,
  title: null,
  durationMs: 120_000,
})

export interface HtmlPlayerOpts {
  /** 서버에 이미 있는 구간 */
  ranges?: PlayRange[]
  settings?: SettingsPatch
  detail?: Partial<AssetDetail>
  /** 자산별 외부 자막 (mock 옵션) */
  subtitles?: MockOptions["subtitles"]
  /** false 면 `loadedmetadata` 를 보내지 않는다 (기본: 보냄 — 자막은 메타데이터가 로드된 뒤에 찾는다) */
  metadata?: boolean
  /** 렌더 전에 api 에 스파이를 거는 훅 */
  setup?: (api: UiApi) => void
  /** 미디어 길이(초) — 트랙 폭 1000px 기준 1px = duration ms */
  duration?: number
  /** 열 때 시각 지정(`openPreview(1, seekMs)`) */
  seekMs?: number
}

/**
 * `<video>`/`<audio>` 경로로 보기 영역을 열고, 구간 command 를 메모리 서버(정규화 = 정렬·병합)로 바꿔 둔다.
 * 레이아웃은 stubLayout 의 1000px 폭 — 길이 100초면 1px = 100ms.
 */
export async function renderHtmlPlayer(opts: HtmlPlayerOpts = {}) {
  resetAllStores()
  stubLayout()
  const frames = stubFrames()
  const media = stubMedia(opts.duration ?? 100)
  // 자동 재생을 켜고 검증하는 테스트는 열기 때의 play() 도 센다 (기본은 stubMedia 설명 참고)
  if (opts.settings?.mediaOpenAutoplay === true) media.state.swallowFirstPlay = false
  const api: UiApi = createMockApi({ count: 20, subtitles: opts.subtitles })
  await useStubbedDetail(api, { mediaType: "video", fileName: "clip.mp4", durationMs: (opts.duration ?? 100) * 1000, ...opts.detail })
  // 기본은 자동 재생 — 정지 상태 설정이면 영상은 포스터가 먼저 뜬다 (VideoPoster.test.tsx). 음성은 설정과 무관
  await api.updateSettings({ mediaOpenAutoplay: true, ...opts.settings })
  opts.setup?.(api)
  // 연속 재생 가드가 대기열을 다시 조회하므로 모의 DB 의 임의 소속이 섞이지 않게 고정한다
  vi.spyOn(api, "collectionPlayQueue").mockResolvedValue([queueItem(1), queueItem(2)])
  let stored: PlayRange[] = mergeRanges(opts.ranges ?? [])
  const get = vi.spyOn(api, "playRangesGet").mockImplementation(async () => structuredClone(stored))
  const set = vi.spyOn(api, "playRangesSet").mockImplementation(async (_id, ranges) => {
    stored = mergeRanges(ranges)
    return structuredClone(stored)
  })
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  // 설정(구간 키 프리셋·열 때 재생 등)이 첫 렌더부터 있도록 미리 받아 둔다
  await queryClient.prefetchQuery({ queryKey: qk.settings, queryFn: () => api.getSettings() })
  const view = render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <input aria-label="아무 입력" />
        <ContentViewer />
      </QueryClientProvider>
    </ApiProvider>
  )
  act(() => useViewStore.getState().openPreview(1, opts.seekMs))
  return { api, media, frames, get, set, queryClient, view, server: () => stored }
}

/** `renderHtmlPlayer` 후 `<video>`/`<audio>` 요소와 메타데이터가 준비될 때까지 기다린다 */
export async function mountHtmlPlayer(opts: HtmlPlayerOpts = {}) {
  const base = await renderHtmlPlayer(opts)
  const label = opts.detail?.fileName ?? "clip.mp4"
  const el = await screen.findByLabelText<HTMLMediaElement>(label, { selector: "video, audio" })
  if (opts.metadata !== false) act(() => void el.dispatchEvent(new Event("loadedmetadata")))
  const lane = await screen.findByTestId("player-track")
  return { ...base, el, lane }
}

/** 트랙 위 포인터 끌기 — 좌표(px)는 트랙 폭 1000px 기준. 기본은 탐색, `shiftKey` 면 새 구간 */
export function dragLane(target: Element, from: number, to: number, lane: Element = target, shiftKey = false) {
  fireEvent.pointerDown(target, { clientX: from, button: 0, pointerId: 1, shiftKey })
  fireEvent.pointerMove(lane, { clientX: to, pointerId: 1 })
  fireEvent.pointerUp(lane, { clientX: to, pointerId: 1 })
}

/** 구간 박스를 우클릭해 선택하고 메뉴는 닫는다 — 구간 선택은 우클릭·키로만 한다(왼쪽 클릭은 탐색) */
export async function selectRangeByContextMenu(box: Element) {
  fireEvent.contextMenu(box)
  fireEvent.keyDown(await screen.findByRole("menu"), { key: "Escape" })
  await waitFor(() => expect(screen.queryByRole("menu")).toBeNull())
}

/** 가짜 타이머를 앞으로 보낸다 (비동기 콜백·react-query 알림 포함) */
export const advance = (ms: number) => act(async () => void (await vi.advanceTimersByTimeAsync(ms)))

/** 타이머만 가짜로 — 초기 렌더(findBy) 뒤에 켠다. react-query 알림도 setTimeout 이라 advance 로 진행한다 */
export const fakeTimers = () => vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
