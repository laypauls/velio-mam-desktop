// Categories: Boundary (frame-aligned / float positions at a range start) / Concurrency (seek storm guard) / Business rule (end, paused-open)
// Evidence: 25 fps H.264, ranges [751-923, 1444-1810, 2785-3399, 3599-3895] ms, ranges-only on. After jumping to range 2 the presented frame
// time (rVFC mediaTime 1.44) stayed below startMs 1444, so every frame re-seeked to 1.444 (97 seeks / 2 s, playback never advanced).
import { renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { PlayRange } from "@/lib/ipc-contract"
import { RESEEK_GUARD_MS, useRangePlayback } from "./useRangePlayback"

const RANGES: PlayRange[] = [
  { startMs: 751, endMs: 923 },
  { startMs: 1444, endMs: 1810 },
  { startMs: 2785, endMs: 3399 },
  { startMs: 3599, endMs: 3895 },
]

/** rVFC 를 스텁한 가짜 `<video>` — 콜백은 직접 발사한다 (고정 턴 flush 로 기다리지 않는다) */
function fakeVideo(init: { paused: boolean; currentTimeSec: number; seeking?: boolean }) {
  const el = document.createElement("video")
  const sets: number[] = []
  const state = { t: init.currentTimeSec, paused: init.paused, seeking: init.seeking ?? false }
  Object.defineProperty(el, "currentTime", {
    get: () => state.t,
    set: (v: number) => {
      sets.push(v)
      state.t = v
    },
  })
  Object.defineProperty(el, "paused", { get: () => state.paused })
  Object.defineProperty(el, "seeking", { get: () => state.seeking })
  let pending: ((now: number, meta: { mediaTime: number }) => void) | null = null
  Object.assign(el, {
    requestVideoFrameCallback: vi.fn((cb: (now: number, meta: { mediaTime: number }) => void) => {
      pending = cb
      return 1
    }),
    cancelVideoFrameCallback: vi.fn(() => {
      pending = null
    }),
  })
  const pause = vi.spyOn(el, "pause").mockImplementation(() => {
    state.paused = true
  })
  /** 프레임 하나가 표시된 것처럼 콜백을 발사한다 */
  const frame = (mediaTimeSec: number) => {
    const cb = pending
    pending = null
    cb?.(0, { mediaTime: mediaTimeSec })
  }
  return { el, sets, state, frame, pause }
}

const mount = (el: HTMLMediaElement, onEnd = vi.fn(), isVideo = true) => {
  renderHook(() => useRangePlayback(el, RANGES, { isVideo, fps: 25, onEnd }))
  return onEnd
}

afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
})

describe("useRangePlayback — start tolerance and re-seek guard", () => {
  it("T-ui-rangeplay-01 [Boundary] frame-aligned mediaTime 1.44 before start 1444 (25 fps) is inside the range: one seek, then none for 100 frames", () => {
    // Arrange: 구간 사이(1.0s)에서 재생 → 구간 2 시작으로 한 번 seek
    const v = fakeVideo({ paused: false, currentTimeSec: 1.0 })
    mount(v.el)
    expect(v.sets).toEqual([1.444])
    // Act: seek 끝(seeking=false) 뒤 표시 프레임은 1.44 — 이전엔 프레임마다 같은 목표로 다시 seek
    for (let i = 0; i < 100; i++) v.frame(1.44)
    // Assert
    expect(v.sets).toEqual([1.444])
  })

  it("T-ui-rangeplay-02 [Boundary] float error 1443.9999 ms (mediaTime and currentTime) never re-seeks", () => {
    // Arrange
    const v = fakeVideo({ paused: false, currentTimeSec: 1.0 })
    mount(v.el)
    v.state.t = 1.4439999
    // Act
    for (let i = 0; i < 50; i++) v.frame(1.4439999)
    // Assert
    expect(v.sets).toEqual([1.444])
  })

  it("T-ui-rangeplay-03 [Concurrency] never seeks while el.seeking is true, even when the position is outside every range", () => {
    // Arrange
    const v = fakeVideo({ paused: false, currentTimeSec: 1.0 })
    mount(v.el)
    v.state.seeking = true
    // Act: 구간 앞 위치가 계속 들어와도
    for (let i = 0; i < 100; i++) v.frame(1.2)
    // Assert
    expect(v.sets).toEqual([1.444])
  })

  it("T-ui-rangeplay-04 [Concurrency] the same target is not re-requested within the guard window, but is retried after it (stuck decoder)", () => {
    // Arrange
    let now = 10_000
    vi.spyOn(performance, "now").mockImplementation(() => now)
    const v = fakeVideo({ paused: false, currentTimeSec: 1.0 })
    mount(v.el)
    // Act 1: 목표 밖(1.2s) 프레임이 가드 시간 안에 계속 온다
    for (let i = 0; i < 20; i++) v.frame(1.2)
    // Assert 1
    expect(v.sets).toEqual([1.444])
    // Act 2: 가드 시간이 지나도 못 갔으면 한 번 더 시도
    now += RESEEK_GUARD_MS + 1
    v.frame(1.2)
    v.frame(1.2)
    // Assert 2
    expect(v.sets).toEqual([1.444, 1.444])
  })

  it("T-ui-rangeplay-05 [Business rule] a position genuinely further than one frame before a start still seeks to that start; a different target is never blocked by the guard", () => {
    // Arrange: 구간 3 안(3.0s)에서 재생
    const v = fakeVideo({ paused: false, currentTimeSec: 3.0 })
    mount(v.el)
    expect(v.sets).toEqual([])
    // Act: 구간 3 끝 직전을 지나 구간 4 앞(3.5s)으로 흘러가면 구간 4 시작으로
    v.frame(3.5)
    // Assert
    expect(v.sets).toEqual([3.599])
  })

  it("T-ui-rangeplay-06 [Business rule] last range end: pauses, parks at the end, reports onEnd exactly once", () => {
    // Arrange
    const v = fakeVideo({ paused: false, currentTimeSec: 3.7 })
    const onEnd = mount(v.el)
    // Act: 마지막 끝 3895 ms 보다 반 프레임(20 ms) 안쪽 프레임
    v.frame(3.88)
    v.frame(3.9)
    // Assert
    expect(v.pause).toHaveBeenCalledTimes(1)
    expect(v.sets).toEqual([3.895])
    expect(onEnd).toHaveBeenCalledTimes(1)
  })

  it("T-ui-rangeplay-07 [Boundary] paused open before the first range seeks to its start; within one frame (751 - 40) or float 750.9999 does not", () => {
    // Arrange + Act + Assert: 멀리 앞
    const far = fakeVideo({ paused: true, currentTimeSec: 0 })
    mount(far.el)
    expect(far.sets).toEqual([0.751])
    // 한 프레임 이내 / 부동소수 오차
    for (const t of [0.711, 0.7509999, 0.751]) {
      const near = fakeVideo({ paused: true, currentTimeSec: t })
      mount(near.el)
      expect(near.sets).toEqual([])
    }
  })

  it("T-ui-rangeplay-08 [Concurrency] paused open does not seek while the element is already seeking (loadedmetadata storm)", () => {
    // Arrange
    const v = fakeVideo({ paused: true, currentTimeSec: 0, seeking: true })
    mount(v.el)
    // Act
    for (let i = 0; i < 5; i++) v.el.dispatchEvent(new Event("loadedmetadata"))
    // Assert
    expect(v.sets).toEqual([])
  })

  it("T-ui-rangeplay-09 [Boundary] audio (rAF path): float 1443.9999 right after seeking to a start never re-seeks", () => {
    // Arrange
    const v = fakeVideo({ paused: false, currentTimeSec: 1.0 })
    Object.assign(v.el, { requestVideoFrameCallback: undefined })
    const raf: { cb: FrameRequestCallback | null } = { cb: null }
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => ((raf.cb = cb), 1))
    vi.stubGlobal("cancelAnimationFrame", () => (raf.cb = null))
    mount(v.el, vi.fn(), false)
    expect(v.sets).toEqual([1.444])
    // Act
    for (let i = 0; i < 20; i++) {
      v.state.t = 1.4439999
      const cb = raf.cb
      raf.cb = null
      cb?.(0)
    }
    // Assert
    expect(v.sets).toEqual([1.444])
  })
})
