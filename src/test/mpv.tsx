import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, render, screen, waitFor } from "@testing-library/react"
import { expect, vi } from "vitest"
import { ContentViewer } from "@/components/viewer/ContentViewer"
import { mergeRanges } from "@/components/viewer/playRanges"
import { qk } from "@/hooks/keys"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi, type MockOptions } from "@/lib/api/mock"
import type { AssetDetail, PlayQueueItem, PlayRange, SettingsPatch } from "@/lib/ipc-contract"
import type { PlayerOpenResult, PlayerStatePayload } from "@/lib/ipc-contract-player"
import type { FrameMsg } from "@/lib/ipc-player-wire"
import { resetAllStores, usePlaybackStore, useViewStore } from "@/stores"
import { stubLayout } from "./layout"
import { queueItem } from "./ranges"
import { installFakeWebgl, stubRafQueue } from "./webgl"
import { installFakeWebView2, makeFrame, type FakeWebView2 } from "./webview2"

export { queueItem as mpvQueueItem }

export interface MpvOpts {
  settings?: SettingsPatch
  ranges?: PlayRange[]
  detail?: Partial<AssetDetail>
  /** "pending" 이면 finishOpen() 을 부를 때까지 player_open 이 끝나지 않는다. 객체면 그 오류로 실패한다 */
  open?: "ok" | "pending" | "fail" | { code: string; message: string }
  /** `player_open` 응답의 상태·시각 (기본 paused, 0) — 응답이 첫 프레임 상태를 알려 준다 */
  reply?: Partial<PlayerOpenResult>
  /** 연속 재생으로 시작할 대기열 (없으면 단독으로 보기 영역을 연다) */
  queue?: PlayQueueItem[]
  /** 자산별 외부 자막 (mock 옵션) */
  subtitles?: MockOptions["subtitles"]
  /** true 면 열기 전에 구간을 캐시에 미리 넣는다 — 열 때 구간을 이미 받은 상태 (player_open 에 실리는 경로) */
  prefetchRanges?: boolean
  /** 열 때 시각 지정(`openPreview(1, seekMs)`) — 포스터 단계를 건너뛰고 바로 연다 */
  seekMs?: number
  /** `player_available` 응답 (기본 true) */
  available?: boolean
  /** true 면 WebGL2 를 쓸 수 없는 환경 (getContext 가 null) */
  noWebgl?: boolean
  /** 열기·닫기 순서를 기록할 배열 ("open:<assetId>" / "close") */
  calls?: string[]
  /** `player_open` 이 호출되는 순간 실행 — 그 시점의 상태(예: 리스너 설치 여부)를 확인한다 */
  onOpen?: (t: { webview: FakeWebView2 }) => void
  /** 무대 크기(CSS px) — 기본은 `stubLayout` 의 1000×600 */
  stageSize?: { width: number; height: number }
  /** 기본 스파이를 건 직후, 렌더 전에 api 를 덮어쓰는 훅 (예: 늦게 끝나는 `playerAvailable`) */
  setup?: (api: UiApi) => void
}

/** 무대 크기 스텁(`stubLayout` 기본 1000×600) 아래, 1920×1080 영상의 contain 크기 (DPR 1) */
export const CONTAIN_1080P = { width: 1000, height: 563 }

/**
 * 내장 플레이어(libmpv) 경로로 보기 영역을 열고 플레이어 IPC 를 전부 스파이로 바꾼다.
 * `webview` = 가짜 `chrome.webview`, `gl` = 가짜 WebGL2, `flushRaf()` = 대기 중 rAF 실행(결정적 스텁, 취소 API 함께).
 * `emitState` 는 구독된 처리기에 이벤트를 보낸다(구독 해제된 뒤에는 전달하지 않는다).
 * `sendFrame` 은 필요하면 에포크 버퍼 3개를 먼저 보내고 프레임 메시지를 보낸 뒤 rAF 를 실행한다.
 */
export async function mountMpv(opts: MpvOpts = {}) {
  resetAllStores()
  stubLayout()
  if (opts.stageSize) {
    const { width, height } = opts.stageSize
    vi.mocked(Element.prototype.getBoundingClientRect).mockReturnValue({ x: 0, y: 0, top: 0, left: 0, right: width, bottom: height, width, height, toJSON: () => ({}) })
  }
  const raf = stubRafQueue()
  const webview = installFakeWebView2()
  const gl = installFakeWebgl({ unsupported: opts.noWebgl })
  const api: UiApi = createMockApi({ count: 20, subtitles: opts.subtitles })
  const base = await api.getAssetDetail(1)
  const detail: AssetDetail = { ...base, mediaType: "video", playable: false, availability: "online", fileName: "v1.mxf", fps: 25, durationMs: 120_000, width: 1920, height: 1080, ...opts.detail }
  vi.spyOn(api, "getAssetDetail").mockImplementation((id) => Promise.resolve({ ...detail, assetId: id, fileName: `v${id}.mxf` }))
  vi.spyOn(api, "mediaUrl").mockImplementation((id) => `media://${id}`)
  // 기본은 자동 재생 — 정지 상태 설정이면 내장 플레이어도 정지로 열린다 (VideoPoster.test.tsx)
  await api.updateSettings({ videoPlayerVlc: true, mediaOpenAutoplay: true, ...opts.settings })
  let stored: PlayRange[] = mergeRanges(opts.ranges ?? [])
  const getRanges = vi.spyOn(api, "playRangesGet").mockImplementation(async () => structuredClone(stored))
  const setRanges = vi.spyOn(api, "playRangesSet").mockImplementation(async (_id, r) => {
    stored = mergeRanges(r)
    return structuredClone(stored)
  })
  vi.spyOn(api, "playerAvailable").mockResolvedValue(opts.available ?? true)
  opts.setup?.(api)
  let finishOpen: () => void = () => undefined
  // 이어보기 조회가 끝난 뒤에야 열기가 시작되므로 finishOpen() 이 player_open 보다 먼저 불릴 수 있다 — 요청을 기억해 두었다가 열기가 시작되면 곧바로 끝낸다
  let finishRequested = false
  const reply: PlayerOpenResult = { sessionId: 1, lengthMs: 120_000, fps: 25, state: "paused", timeMs: 0, ...opts.reply }
  const open = vi.spyOn(api, "playerOpen").mockImplementation(async (id) => {
    opts.calls?.push(`open:${id}`)
    opts.onOpen?.({ webview })
    if (opts.open === "pending" && !finishRequested) await new Promise<void>((r) => (finishOpen = r))
    if (opts.open === "fail") throw { code: "open_failed", message: "x" }
    if (typeof opts.open === "object") throw opts.open
    return reply
  })
  const setPlayerRanges = vi.spyOn(api, "playerSetRanges").mockResolvedValue(undefined)
  const setSubtitle = vi.spyOn(api, "playerSetSubtitle").mockResolvedValue(undefined)
  const control = vi.spyOn(api, "playerControl").mockResolvedValue(undefined)
  const seek = vi.spyOn(api, "playerSeek").mockResolvedValue(undefined)
  const setVolume = vi.spyOn(api, "playerSetVolume").mockResolvedValue(undefined)
  const setRate = vi.spyOn(api, "playerSetRate").mockResolvedValue(undefined)
  const setRenderSize = vi.spyOn(api, "playerSetRenderSize").mockResolvedValue(undefined)
  const setRenderActive = vi.spyOn(api, "playerSetRenderActive").mockResolvedValue(undefined)
  const releaseSlots = vi.spyOn(api, "playerReleaseSlots").mockImplementation(async (msg) => webview.recordRelease(msg))
  const restart = vi.spyOn(api, "playerRestart").mockResolvedValue({ ...reply, sessionId: 2, state: "playing" })
  const close = vi.spyOn(api, "playerClose").mockImplementation(async () => void opts.calls?.push("close"))
  const openOriginal = vi.spyOn(api, "openOriginal").mockResolvedValue(undefined)
  let stateHandler: ((p: PlayerStatePayload) => void) | null = null
  vi.spyOn(api, "onPlayerState").mockImplementation(async (h) => {
    stateHandler = h
    return () => {
      if (stateHandler === h) stateHandler = null
    }
  })
  if (opts.queue) vi.spyOn(api, "collectionPlayQueue").mockResolvedValue(opts.queue)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  await queryClient.prefetchQuery({ queryKey: qk.settings, queryFn: () => api.getSettings() })
  if (opts.prefetchRanges) await queryClient.prefetchQuery({ queryKey: qk.playRanges(1), queryFn: () => api.playRangesGet(1), staleTime: Number.POSITIVE_INFINITY })
  const view = render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <ContentViewer />
      </QueryClientProvider>
    </ApiProvider>
  )
  if (opts.queue) act(() => usePlaybackStore.getState().start(1, opts.queue ?? []))
  else act(() => useViewStore.getState().openPreview(1, opts.seekMs))
  const stage = () => screen.getByRole("img", { name: "v1.mxf", hidden: true })
  const canvas = () => stage().querySelector("canvas") as HTMLCanvasElement
  // aria-busy 가 false 가 되는 커밋과, 열기 응답을 재생 상태에 채우는 효과(useMpvPlayback)는 별개 단계다 — 부하 상태에서는 waitFor 가 커밋만 보고 돌아와
  // 이후 emitState 가 효과보다 먼저 반영되고 늦은 응답이 그 값을 덮어쓴다. 응답 반영이 끝나도록 act 로 대기 중인 효과·갱신을 비운다
  const ready = async () => {
    await waitFor(() => expect(stage()).toHaveAttribute("aria-busy", "false"))
    await act(async () => {})
  }
  const banks = new Set<number>()
  return {
    api, view, queryClient, detail, stage, canvas, ready,
    open, setPlayerRanges, setSubtitle, control, seek, setVolume, setRate, setRenderSize, setRenderActive, releaseSlots, restart, close, openOriginal, getRanges, setRanges,
    webview, gl, raf, flushRaf: () => act(() => raf.flush()),
    finishOpen: () =>
      act(() => {
        finishRequested = true
        finishOpen()
      }),
    server: () => stored,
    subscribedToState: () => stateHandler !== null,
    emitState: (p: Partial<PlayerStatePayload> & Pick<PlayerStatePayload, "state">) =>
      act(() => stateHandler?.({ sessionId: 1, assetId: 1, timeMs: 0, lengthMs: 120_000, fps: 25, ...p })),
    /** 호스트가 에포크 `epoch` 의 버퍼 3개(첫 호출에만)와 프레임 1장을 보내고 rAF 를 실행한다 */
    sendFrame: (over: Partial<FrameMsg> = {}) =>
      act(() => {
        const epoch = over.epoch ?? 1
        if (!banks.has(epoch)) {
          banks.add(epoch)
          webview.emitBank(epoch, { w: over.w ?? 1000, h: over.h ?? 563 })
        }
        webview.emitFrame(makeFrame({ w: 1000, h: 563, ...over }))
        raf.flush()
      }),
    /** 그리기와 microtask(release 전송)가 끝나도록 한 번 비운다 */
    settle: () => act(async () => void (await Promise.resolve())),
  }
}
