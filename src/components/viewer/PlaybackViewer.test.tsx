// Categories: Happy (T-ui-play-13, 14, 20) / Business rule (T-ui-play-17..19, 21, 22) / External (T-ui-play-15, 23) / Validation (T-ui-play-16)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider, type UiApi } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import type { AssetDetail, PlayQueueItem } from "@/lib/ipc-contract"
import type { PlayerStatePayload } from "@/lib/ipc-contract-player"
import { resetAllStores, usePlaybackStore, useViewStore } from "@/stores"
import { stubLayout } from "@/test/layout"
import { stubMedia, useStubbedDetail } from "@/test/media"
import { mountMpv, mpvQueueItem } from "@/test/mpv"
import { ContentViewer } from "./ContentViewer"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

beforeEach(() => resetAllStores())
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

const q = (assetId: number, mediaType: PlayQueueItem["mediaType"] = "video", availability: PlayQueueItem["availability"] = "online"): PlayQueueItem => ({
  assetId,
  mediaType,
  availability,
  fileName: `item${assetId}.mp4`,
  title: null,
  durationMs: 60_000,
})
const play = () => usePlaybackStore.getState()

/** 보기 영역만 렌더 — 대기열 조회는 `queue` 로 고정한다 (모의 DB 의 임의 소속을 쓰지 않도록) */
function mount(api: UiApi, queue: PlayQueueItem[]) {
  vi.spyOn(api, "collectionPlayQueue").mockResolvedValue(queue)
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } })
  const view = render(
    <ApiProvider api={api}>
      <QueryClientProvider client={queryClient}>
        <input aria-label="아무 입력" />
        <ContentViewer />
      </QueryClientProvider>
    </ApiProvider>
  )
  return { view, queryClient }
}

async function htmlSetup(queue: PlayQueueItem[], patch: Partial<AssetDetail> = {}) {
  stubLayout()
  const media = stubMedia()
  const api = createMockApi({ count: 20 })
  await useStubbedDetail(api, patch)
  const mounted = mount(api, queue)
  act(() => play().start(1, queue))
  return { api, media, ...mounted }
}

describe("continuous playback in the content viewer (HTML media)", () => {
  it("T-ui-play-13 [Happy] a video autoplays when it opens; its ended event moves to the next item (new element, new preview)", async () => {
    // Arrange
    const { media } = await htmlSetup([q(1), q(2)])
    const first = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    await waitFor(() => expect(media.play).toHaveBeenCalled())
    // Act
    fireEvent.ended(first)
    // Assert
    await waitFor(() => expect(play().currentId).toBe(2))
    expect(useViewStore.getState().previewId).toBe(2)
    await waitFor(() => expect(first).not.toBeInTheDocument()) // token 키로 새로 마운트
    expect(screen.getByText("2 / 2")).toBeInTheDocument()
  })

  it("T-ui-play-15 [External] a media error skips the item with a toast and plays the following one", async () => {
    // Arrange
    await htmlSetup([q(1), q(2), q(3)])
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Act
    fireEvent.error(video)
    // Assert
    await waitFor(() => expect(play().currentId).toBe(2))
    expect(play().failed.has(1)).toBe(true)
    expect(toast.warning).toHaveBeenCalledWith("재생할 수 없어 건너뜁니다: item1.mp4")
  })

  it("T-ui-play-17 [Business rule] the bar shows n / total; the last ended with repeat off stops and keeps the viewer on it", async () => {
    // Arrange
    await htmlSetup([q(1), q(2)])
    const video = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    expect(screen.getByText("1 / 2")).toBeInTheDocument()
    fireEvent.ended(video)
    await waitFor(() => expect(play().currentId).toBe(2))
    const second = await screen.findByLabelText<HTMLVideoElement>("clip.mp4", { selector: "video" })
    // Act
    fireEvent.ended(second)
    // Assert
    await waitFor(() => expect(play().status).toBe("stopped"))
    expect(useViewStore.getState().previewId).toBe(2)
    expect(screen.getByText("재생을 마쳤습니다.")).toBeInTheDocument()
  })

  it("T-ui-play-18 [Business rule] closing the viewer stops playback and removes the controls", async () => {
    // Arrange
    const user = userEvent.setup()
    await htmlSetup([q(1), q(2)])
    await screen.findByRole("group", { name: "연속 재생" })
    // Act
    await user.click(screen.getByRole("button", { name: "보기 영역 닫기" }))
    // Assert
    expect(play().status).toBe("idle")
    expect(screen.queryByRole("group", { name: "연속 재생" })).not.toBeInTheDocument()
  })

  it("T-ui-play-19 [Business rule, C4] explicit preview of a queue member jumps to it; of a non-member stops playback and previews normally", async () => {
    // Arrange
    await htmlSetup([q(1), q(2), q(3)])
    await screen.findByRole("group", { name: "연속 재생" })
    // Act 1: 대기열 항목 미리보기
    act(() => useViewStore.getState().openPreview(3))
    // Assert 1
    expect(play()).toMatchObject({ currentId: 3, status: "playing" })
    expect(useViewStore.getState().previewId).toBe(3)
    // Act 2: 대기열 밖
    act(() => useViewStore.getState().openPreview(15))
    // Assert 2
    expect(play().status).toBe("idle")
    expect(useViewStore.getState().previewId).toBe(15)
    await waitFor(() => expect(screen.queryByRole("group", { name: "연속 재생" })).not.toBeInTheDocument())
  })

  it("T-ui-play-20 [Happy] clicking a queue row jumps; offline items are dimmed with their state and skipped by next", async () => {
    // Arrange
    const user = userEvent.setup()
    await htmlSetup([q(1), q(2, "video", "offline"), q(3)])
    const list = await screen.findByRole("list", { name: "재생 대기열" })
    expect(within(list).getByText("오프라인")).toBeInTheDocument()
    // Act
    await user.click(within(list).getByRole("button", { name: /item3\.mp4/ }))
    // Assert
    expect(play().currentId).toBe(3)
    expect(within(list).getByRole("button", { name: /item3\.mp4/ })).toHaveAttribute("aria-current", "true")
    // 이전 → 오프라인 2 를 건너뛰어 1
    await user.click(screen.getByRole("button", { name: "이전 항목" }))
    expect(play().currentId).toBe(1)
  })

  it("T-ui-play-16 [Validation] Shift+N / Shift+P move while playback is active, never inside inputs or with Ctrl, and not after it stops", async () => {
    // Arrange
    const user = userEvent.setup()
    await htmlSetup([q(1), q(2), q(3)])
    await screen.findByRole("group", { name: "연속 재생" })
    // Act + Assert: 다음 / 이전
    await user.keyboard("{Shift>}N{/Shift}")
    expect(play().currentId).toBe(2)
    await user.keyboard("{Shift>}p{/Shift}")
    expect(play().currentId).toBe(1)
    // 입력 요소 안에서는 무시
    await user.click(screen.getByRole("textbox", { name: "아무 입력" }))
    await user.keyboard("{Shift>}N{/Shift}")
    expect(play().currentId).toBe(1)
    // Ctrl 이 함께면 무시
    ;(document.activeElement as HTMLElement).blur()
    await user.keyboard("{Control>}{Shift>}N{/Shift}{/Control}")
    expect(play().currentId).toBe(1)
    // 정지 후에는 리스너가 없다
    act(() => play().stop())
    await user.keyboard("{Shift>}N{/Shift}")
    expect(play().status).toBe("idle")
  })

  it("T-ui-play-21 [Business rule] the queue is refetched when the collection changes and the current position follows", async () => {
    // Arrange
    const { api, queryClient } = await htmlSetup([q(1), q(2), q(3)])
    await screen.findByRole("group", { name: "연속 재생" })
    await waitFor(() => expect(api.collectionPlayQueue).toHaveBeenCalled())
    // Act: 컬렉션 순서가 바뀌었다 (현재 1 이 맨 뒤로)
    vi.spyOn(api, "collectionPlayQueue").mockResolvedValue([q(2), q(3), q(1)])
    await act(() => queryClient.invalidateQueries({ queryKey: ["collections"] }))
    // Assert
    await waitFor(() => expect(play()).toMatchObject({ currentId: 1, index: 2 }))
    expect(screen.getByText("3 / 3")).toBeInTheDocument()
  })

  it("T-ui-play-23 [External] a deleted collection (not_found on refetch) stops playback", async () => {
    // Arrange
    const { api, queryClient } = await htmlSetup([q(1), q(2)])
    await screen.findByRole("group", { name: "연속 재생" })
    // Act
    vi.spyOn(api, "collectionPlayQueue").mockRejectedValue({ code: "not_found", message: "collection not found" })
    await act(() => queryClient.invalidateQueries({ queryKey: ["collections"] }))
    // Assert
    await waitFor(() => expect(play().status).toBe("idle"))
  })
})

describe("continuous playback with the built-in player (libmpv)", () => {
  async function mpvSetup(queue: PlayQueueItem[]) {
    const t = await mountMpv({ queue: queue.map((i) => mpvQueueItem(i.assetId)), reply: { lengthMs: 60_000 }, detail: { durationMs: 60_000 } })
    await t.ready()
    return (assetId: number, state: PlayerStatePayload["state"], error?: string) => t.emitState({ assetId, state, lengthMs: 60_000, ...(error ? { error } : {}) })
  }

  it("T-ui-play-14 [Happy] an ended event for the playing asset advances; an event for another asset is ignored", async () => {
    // Arrange
    const emit = await mpvSetup([q(1), q(2)])
    // Act 1: 다른 자산의 종료는 무시
    emit(99, "ended")
    expect(play().currentId).toBe(1)
    // Act 2: 현재 자산의 종료
    emit(1, "ended")
    // Assert
    await waitFor(() => expect(play().currentId).toBe(2))
    expect(useViewStore.getState().previewId).toBe(2)
  })

  it("T-ui-play-15b [External] an error event skips the item (marks it failed) and moves on", async () => {
    // Arrange
    const emit = await mpvSetup([q(1), q(2)])
    // Act
    emit(1, "error", "decode failed")
    // Assert
    await waitFor(() => expect(play().currentId).toBe(2))
    expect(play().failed.has(1)).toBe(true)
  })
})
