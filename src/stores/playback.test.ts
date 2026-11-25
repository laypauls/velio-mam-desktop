// Categories: Happy (T-ui-play-01, 04, 06) / Business rule (T-ui-play-02, 05, 09, 10, 11) / Boundary (T-ui-play-03, 12) / External (T-ui-play-07, 08)
import { beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import type { PlayQueueItem } from "@/lib/ipc-contract"
import { toast } from "sonner"
import { resetAllStores, usePlaybackStore, useViewStore } from "@/stores"

vi.mock("sonner", () => ({ toast: { error: vi.fn(), warning: vi.fn(), info: vi.fn(), success: vi.fn() } }))

const item = (assetId: number, availability: PlayQueueItem["availability"] = "online", mediaType: PlayQueueItem["mediaType"] = "video"): PlayQueueItem => ({
  assetId,
  mediaType,
  availability,
  fileName: `f${assetId}.mp4`,
  title: null,
  durationMs: 1000,
})
const Q = [item(1), item(2), item(3), item(4)]
const play = () => usePlaybackStore.getState()
const preview = () => useViewStore.getState().previewId

beforeEach(() => {
  resetAllStores()
  vi.clearAllMocks()
})

describe("playback store", () => {
  it("T-ui-play-01 [Happy] start from an item plays it, opens the preview, and bumps the token", () => {
    // Arrange + Act
    play().start(7, Q, 3)
    // Assert
    expect(play()).toMatchObject({ collectionId: 7, currentId: 3, index: 2, status: "playing", token: 1 })
    expect(preview()).toBe(3)
  })

  it("T-ui-play-02 [Business rule] offline and missing items are skipped on start, next and prev", () => {
    // Arrange
    const q = [item(1), item(2, "offline"), item(3, "missing"), item(4)]
    // Act + Assert: 시작
    play().start(1, q)
    expect(play().currentId).toBe(1)
    play().next()
    expect(play().currentId).toBe(4)
    play().prev()
    expect(play().currentId).toBe(1)
  })

  it("T-ui-play-03 [Boundary] nothing playable (all offline) toasts noPlayable and stays idle", () => {
    // Arrange + Act
    play().start(1, [item(1, "offline"), item(2, "missing")])
    // Assert
    expect(play().status).toBe("idle")
    expect(toast.error).toHaveBeenCalledWith("재생할 수 있는 항목이 없습니다.")
    expect(preview()).toBeNull()
  })

  it("T-ui-play-04 [Happy] ended on the current item advances; an ended report for another item is ignored", () => {
    // Arrange
    play().start(1, Q)
    // Act + Assert
    play().reportEnded(2)
    expect(play().currentId).toBe(1)
    play().reportEnded(1)
    expect(play().currentId).toBe(2)
    expect(preview()).toBe(2)
  })

  it("T-ui-play-05 [Business rule] repeat off stops after the last item and keeps the viewer on it; prev at the start is a no-op", () => {
    // Arrange
    play().start(1, Q)
    play().prev()
    expect(play().currentId).toBe(1)
    play().jumpTo(4)
    // Act
    play().reportEnded(4)
    // Assert
    expect(play().status).toBe("stopped")
    expect(play().currentId).toBe(4)
    expect(preview()).toBe(4)
  })

  it("T-ui-play-06 [Happy] repeat all wraps to the first playable item", () => {
    // Arrange
    play().toggleRepeat()
    play().start(1, [item(1, "offline"), item(2), item(3)], 3)
    // Act
    play().reportEnded(3)
    // Assert: 1 은 오프라인이라 2 로
    expect(play().currentId).toBe(2)
    expect(play().status).toBe("playing")
  })

  it("T-ui-play-07 [External] a playback error marks the item failed, toasts, and moves on", () => {
    // Arrange
    play().start(1, Q)
    // Act
    play().reportFailed(1)
    // Assert
    expect(toast.warning).toHaveBeenCalledWith("재생할 수 없어 건너뜁니다: f1.mp4")
    expect(play().currentId).toBe(2)
    expect(play().failed.has(1)).toBe(true)
    // 반복 전체로 한 바퀴 돌아도 실패 항목은 다시 재생하지 않는다
    play().toggleRepeat()
    play().jumpTo(4)
    play().reportEnded(4)
    expect(play().currentId).toBe(2)
  })

  it("T-ui-play-08 [External] a run of failures covering the whole queue stops with noPlayable", () => {
    // Arrange
    const q = [item(1), item(2), item(3)]
    play().toggleRepeat()
    play().start(1, q)
    // Act
    play().reportFailed(1)
    play().reportFailed(2)
    play().reportFailed(3)
    // Assert
    expect(play().status).toBe("idle")
    expect(toast.error).toHaveBeenCalledWith("재생할 수 있는 항목이 없습니다.")
  })

  it("T-ui-play-09 [Business rule] a queue refetch relocates the current item; if it vanished, next plays the item that took its slot", () => {
    // Arrange
    play().start(1, Q, 2)
    // Act 1: 순서 변경 — 현재 항목(2)이 뒤로
    play().setQueue([item(1), item(3), item(2), item(4)])
    // Assert 1
    expect(play()).toMatchObject({ currentId: 2, index: 2 })
    // Act 2: 현재 항목(2)이 컬렉션에서 빠짐 → 원래 다음이던 4
    play().setQueue([item(1), item(3), item(4)])
    play().next()
    // Assert 2
    expect(play().currentId).toBe(4)
  })

  it("T-ui-play-09b [Business rule] the removed current item's slot passes to the item that followed it in the middle of the queue", () => {
    // Arrange: 현재 2 (index 1)
    play().start(1, Q, 2)
    // Act: 2 가 빠지면 index 1 자리를 3 이 이어받는다
    play().setQueue([item(1), item(3), item(4)])
    play().next()
    // Assert
    expect(play().currentId).toBe(3)
  })

  it("T-ui-play-10 [Business rule] stop clears the state; reports after stop are ignored; setQueue while idle does nothing", () => {
    // Arrange
    play().start(1, Q)
    // Act
    play().stop()
    play().reportEnded(1)
    play().setQueue(Q)
    // Assert
    expect(play()).toMatchObject({ status: "idle", currentId: null, collectionId: null })
    expect(play().queue).toEqual([])
  })

  it("T-ui-play-11 [Business rule, C4] clicks: plain click on a member jumps, Ctrl/Shift and non-members do not, the playing card is not restarted", () => {
    // Arrange
    play().start(1, Q)
    const token = play().token
    // Act + Assert: Ctrl / Shift
    play().clickAsset(3, { ctrl: true, shift: false })
    play().clickAsset(3, { ctrl: false, shift: true })
    expect(play().currentId).toBe(1)
    // 대기열 밖 카드
    play().clickAsset(99, { ctrl: false, shift: false })
    expect(play().currentId).toBe(1)
    // 지금 재생 중인 카드
    play().clickAsset(1, { ctrl: false, shift: false })
    expect(play().token).toBe(token)
    // 일반 클릭 + 대기열 항목
    play().clickAsset(3, { ctrl: false, shift: false })
    expect(play().currentId).toBe(3)
    expect(preview()).toBe(3)
  })

  it("T-ui-play-11b [Business rule] clicks do nothing while nothing is playing", () => {
    // Arrange + Act
    play().clickAsset(3, { ctrl: false, shift: false })
    // Assert
    expect(play().status).toBe("idle")
    expect(preview()).toBeNull()
  })

  it("T-ui-play-12 [Boundary] a single-item queue with repeat all re-selects it with a new token each time", () => {
    // Arrange
    play().toggleRepeat()
    play().start(1, [item(1)])
    const t0 = play().token
    // Act
    play().reportEnded(1)
    // Assert
    expect(play().currentId).toBe(1)
    expect(play().token).toBe(t0 + 1)
    expect(play().status).toBe("playing")
  })

  it("T-ui-play-12b [Business rule] pausing only toggles between playing and paused (image timer)", () => {
    // Arrange
    play().start(1, [item(1, "online", "image")])
    // Act + Assert
    play().togglePause()
    expect(play().status).toBe("paused")
    play().togglePause()
    expect(play().status).toBe("playing")
    play().stop()
    play().togglePause()
    expect(play().status).toBe("idle")
  })
})
