// Categories: Business rule (T-ui-sub-05 sent pattern + order, T-ui-sub-06 collection language) / External (T-ui-sub-08 player_set_subtitle failure)
import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { act, renderHook, screen, waitFor, within } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import type { ReactNode } from "react"
import { toast } from "sonner"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { ApiProvider } from "@/lib/api"
import { createMockApi } from "@/lib/api/mock"
import { usePlaybackStore } from "@/stores"
import { subEntry, subTrack } from "@/test/subtitles"
import { mountMpv, mpvQueueItem } from "@/test/mpv"
import { useMpvSubtitle } from "./useMpvSession"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
  vi.mocked(toast.error).mockClear()
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

type Props = { ready: boolean; id: string | null | undefined }
const both = (id: number) => [subEntry(subTrack(`ko${id}`, "ko")), subEntry(subTrack(`en${id}`, "en"))]

describe("built-in player external subtitles", () => {
  it("T-ui-sub-05 [Business rule] after open, the picked track is sent once; menu changes and 끄기 are sent; nothing is sent before a track exists", async () => {
    // Arrange
    const user = userEvent.setup()
    const t = await mountMpv({ subtitles: { 1: both(1) } })
    // Act + Assert: 자동 선택(ko)이 열기 뒤 1회
    await t.ready()
    await waitFor(() => expect(t.setSubtitle).toHaveBeenCalledTimes(1))
    expect(t.setSubtitle).toHaveBeenLastCalledWith(1, "ko1")
    // Act: 영어
    await user.click(await screen.findByRole("button", { name: "자막" }))
    await user.click(await screen.findByRole("menuitemradio", { name: "영어 (SRT)" }))
    // Assert
    await waitFor(() => expect(t.setSubtitle).toHaveBeenLastCalledWith(1, "en1"))
    expect(t.setSubtitle).toHaveBeenCalledTimes(2)
    // Act: 끄기
    await user.click(screen.getByRole("button", { name: "자막" }))
    await user.click(await screen.findByRole("menuitemradio", { name: "자막 끄기" }))
    // Assert
    await waitFor(() => expect(t.setSubtitle).toHaveBeenLastCalledWith(1, null))
    expect(t.setSubtitle).toHaveBeenCalledTimes(3)
    t.view.unmount()
    // Arrange + Act: 자막 없는 영상 — 아무것도 보내지 않는다
    const none = await mountMpv()
    await none.ready()
    // Assert
    expect(await screen.findByRole("button", { name: "자막" })).toBeDisabled()
    expect(none.setSubtitle).not.toHaveBeenCalled()
  })

  it("T-ui-sub-05 [Business rule] useMpvSubtitle: re-open re-sends, unknown (undefined) sends nothing, off after a track sends null, and calls are serialized", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    const gates: (() => void)[] = []
    const calls: (string | null)[] = []
    vi.spyOn(api, "playerSetSubtitle").mockImplementation(
      (_id, track) =>
        new Promise<void>((resolve) => {
          calls.push(track)
          gates.push(resolve)
        })
    )
    const wrapper = ({ children }: { children: ReactNode }) => (
      <ApiProvider api={api}>
        <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
      </ApiProvider>
    )
    const flush = () => act(async () => void (await Promise.resolve()))
    const { rerender } = renderHook<void, Props>((p) => useMpvSubtitle(1, p.ready, p.id), {
      wrapper,
      initialProps: { ready: false, id: "a" },
    })
    // Act + Assert: 열리기 전에는 보내지 않는다
    await flush()
    expect(calls).toEqual([])
    // Act: 열림
    rerender({ ready: true, id: "a" })
    await flush()
    expect(calls).toEqual(["a"])
    // Act: 앞선 요청이 끝나기 전의 변경은 기다린다
    rerender({ ready: true, id: "b" })
    await flush()
    expect(calls).toEqual(["a"])
    await act(async () => gates[0]())
    await waitFor(() => expect(calls).toEqual(["a", "b"]))
    await act(async () => gates[1]())
    // Act: 모름(undefined) → 보내지 않음, 끔 → null
    rerender({ ready: true, id: undefined })
    await flush()
    expect(calls).toHaveLength(2)
    rerender({ ready: true, id: null })
    await waitFor(() => expect(calls).toEqual(["a", "b", null]))
    await act(async () => gates[2]())
    // Act: 닫힘 → 다시 열림: 플레이어가 자막을 잊었으므로 고른 트랙을 다시 보낸다
    rerender({ ready: false, id: "b" })
    await flush()
    rerender({ ready: true, id: "b" })
    await waitFor(() => expect(calls).toEqual(["a", "b", null, "b"]))
    await act(async () => gates[3]())
    // Act: 같은 값으로 다시 렌더 → 재전송 없음
    rerender({ ready: true, id: "b" })
    await flush()
    expect(calls).toHaveLength(4)
  })

  it("T-ui-sub-05 [External] a rejected player_set_subtitle is not remembered as sent: the player still has its old value, and the same selection is sent again later", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    const calls: (string | null)[] = []
    let fail = true
    vi.spyOn(api, "playerSetSubtitle").mockImplementation(async (_id, track) => {
      calls.push(track)
      if (fail) throw { code: "not_found", message: "x" }
    })
    const wrapper = ({ children }: { children: ReactNode }) => (
      <ApiProvider api={api}>
        <QueryClientProvider client={new QueryClient()}>{children}</QueryClientProvider>
      </ApiProvider>
    )
    const { rerender } = renderHook<void, Props>((p) => useMpvSubtitle(1, p.ready, p.id), { wrapper, initialProps: { ready: true, id: "a" } })
    await waitFor(() => expect(toast.error).toHaveBeenCalledTimes(1))
    // Act: 플레이어는 여전히 꺼진 상태 — 끔으로 바뀌어도 보낼 것이 없다
    rerender({ ready: true, id: null })
    await act(async () => void (await Promise.resolve()))
    // Assert
    expect(calls).toEqual(["a"])
    // Act: 같은 선택이 다시 오면 다시 보낸다
    fail = false
    rerender({ ready: true, id: "a" })
    // Assert
    await waitFor(() => expect(calls).toEqual(["a", "a"]))
    expect(toast.error).toHaveBeenCalledTimes(1)
  })

  it("T-ui-sub-06 [Business rule] collection playback: English picked on item 1 is auto-picked on item 2 (D10); an item without it falls back to the default language", async () => {
    // Arrange
    const user = userEvent.setup()
    const queue = [mpvQueueItem(1), mpvQueueItem(2), mpvQueueItem(4)]
    const t = await mountMpv({
      queue,
      subtitles: { 1: both(1), 2: both(2), 4: [subEntry(subTrack("ja4", "ja")), subEntry(subTrack("ko4", "ko"))] },
    })
    await waitFor(() => expect(t.setSubtitle).toHaveBeenCalledWith(1, "ko1"))
    // Act: 1번에서 영어 선택
    await user.click(await screen.findByRole("button", { name: "자막" }))
    await user.click(await screen.findByRole("menuitemradio", { name: "영어 (SRT)" }))
    await waitFor(() => expect(t.setSubtitle).toHaveBeenLastCalledWith(1, "en1"))
    // Act: 2번으로
    act(() => usePlaybackStore.getState().next())
    // Assert
    await waitFor(() => expect(t.setSubtitle).toHaveBeenCalledWith(2, "en2"))
    expect(t.setSubtitle).not.toHaveBeenCalledWith(2, "ko2")
    // Act: 4번 — 영어 없음 (큐의 3번째 항목)
    act(() => usePlaybackStore.getState().next())
    // Assert: 기본 언어(ko)
    await waitFor(() => expect(t.setSubtitle).toHaveBeenCalledWith(4, "ko4"))
  })

  it("T-ui-sub-08 [External] player_set_subtitle failing → one toast and the player stays", async () => {
    // Arrange
    const t = await mountMpv({ subtitles: { 1: both(1) } })
    t.setSubtitle.mockRejectedValue({ code: "not_found", message: "x" })
    // Act
    await t.ready()
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith("자막을 불러오지 못했습니다."))
    // Assert
    expect(toast.error).toHaveBeenCalledTimes(1)
    expect(t.stage()).toBeInTheDocument()
    expect(within(document.body).queryByRole("alert")).not.toBeInTheDocument()
  })
})
