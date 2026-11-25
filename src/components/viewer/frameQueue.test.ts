// Categories: Happy (draw newest, swap epoch) / Business rule (releaseBuffer rules, staging) / Boundary (stale epoch, incomplete bank, oversize frame)
//             External (missing buffer, malformed frame) / Concurrency (superseded frames, rendering toggled while pending)
import { describe, expect, it } from "vitest"
import { makeBufferInfo, makeFrame } from "@/test/webview2"
import { FrameQueue, groupReleases } from "./frameQueue"

/** 에포크 `epoch` 의 세 버퍼를 먼저 도착시킨다 (버퍼는 문자열 이름으로 구분) */
function withBank(q: FrameQueue<string>, epoch: number, over: Parameters<typeof makeBufferInfo>[0] = {}) {
  for (const slot of [0, 1, 2]) q.onBuffer(makeBufferInfo({ ...over, epoch, slot }), `e${epoch}s${slot}`)
}

/** 첫 에포크를 열고 첫 프레임까지 그려 `current` 로 확정한 큐 */
function opened(): FrameQueue<string> {
  const q = new FrameQueue<string>()
  withBank(q, 1)
  q.onFrame(makeFrame({ epoch: 1, slot: 0, seq: 1 }))
  const job = q.takeDraw()
  if (!job) throw new Error("no first draw")
  q.drawn(job)
  q.drainReleases()
  return q
}

describe("FrameQueue — current epoch", () => {
  it("T-ui-mpv-frames-01 [Happy] the first frame of the first epoch is a swap job that is drawn and then released", () => {
    // Arrange
    const q = new FrameQueue<string>()
    withBank(q, 1)
    // Act
    q.onFrame(makeFrame({ epoch: 1, slot: 1, seq: 5 }))
    const job = q.takeDraw()
    // Assert
    expect(job).toMatchObject({ swap: true, buffer: "e1s1", frame: { slot: 1, seq: 5 } })
    expect(q.currentEpoch).toBe(0) // 그려지기 전에는 확정되지 않는다
    if (job) q.drawn(job)
    expect(q.currentEpoch).toBe(1)
    expect(q.drainReleases()).toEqual([{ epoch: 1, slot: 1, seq: 5 }])
    expect(q.takeDraw()).toBeNull()
  })

  it("T-ui-mpv-frames-02 [Concurrency] only the newest pending frame is drawn; the superseded one is released right away", () => {
    // Arrange
    const q = opened()
    // Act
    q.onFrame(makeFrame({ slot: 0, seq: 2 }))
    q.onFrame(makeFrame({ slot: 1, seq: 3 }))
    // Assert
    expect(q.drainReleases()).toEqual([{ epoch: 1, slot: 0, seq: 2 }])
    expect(q.takeDraw()).toMatchObject({ swap: false, frame: { seq: 3 } })
  })

  it("T-ui-mpv-frames-03 [External] a frame whose slot buffer never arrived is dropped and released, nothing is drawn", () => {
    // Arrange
    const q = new FrameQueue<string>()
    q.onBuffer(makeBufferInfo({ epoch: 1, slot: 0 }), "e1s0")
    q.onBuffer(makeBufferInfo({ epoch: 1, slot: 1 }), "e1s1")
    // Act: 슬롯 2 버퍼 없음 → 뱅크가 덜 찼으므로 첫 에포크 프레임도 거부된다
    q.onFrame(makeFrame({ epoch: 1, slot: 2, seq: 1 }))
    // Assert
    expect(q.takeDraw()).toBeNull()
    expect(q.drainReleases()).toEqual([{ epoch: 1, slot: 2, seq: 1 }])
  })

  it("T-ui-mpv-frames-04 [External] a frame with a different stride, an oversize width/height or a short row is dropped and released", () => {
    // Arrange
    const q = opened()
    const bad = [
      makeFrame({ slot: 0, seq: 2, w: 1000, stride: 4032 }), // stride 불일치 — 다른 검사(행 길이·범위)는 모두 통과한다
      makeFrame({ slot: 1, seq: 3, w: 1281 }), // 뱅크보다 넓다
      makeFrame({ slot: 2, seq: 4, h: 721 }), // 뱅크보다 높다
    ]
    // Act
    for (const f of bad) q.onFrame(f)
    // Assert
    expect(q.takeDraw()).toBeNull()
    expect(q.drainReleases().map((r) => r.seq)).toEqual([2, 3, 4])
  })

  it("T-ui-mpv-frames-05 [Boundary] a frame of an older epoch is ignored without a release", () => {
    // Arrange
    const q = opened()
    withBank(q, 2)
    q.onFrame(makeFrame({ epoch: 2, slot: 0, seq: 1 }))
    const swap = q.takeDraw()
    if (swap) q.drawn(swap)
    q.drainReleases()
    // Act: 에포크 2 가 확정된 뒤 늦게 온 에포크 1 프레임
    q.onFrame(makeFrame({ epoch: 1, slot: 1, seq: 9 }))
    // Assert
    expect(q.currentEpoch).toBe(2)
    expect(q.takeDraw()).toBeNull()
    expect(q.drainReleases()).toEqual([])
  })

  it("T-ui-mpv-frames-06 [Boundary] a late buffer of an older epoch is freed at once, a duplicate or mismatching slot buffer too", () => {
    // Arrange
    const q = opened()
    q.drainFreed()
    // Act
    q.onBuffer(makeBufferInfo({ epoch: 0 + 1, slot: 0 }), "dup-current") // 현재 에포크의 이미 찬 슬롯
    withBank(q, 3)
    q.onBuffer(makeBufferInfo({ epoch: 2, slot: 0 }), "late-e2") // 3 이 스테이징 중이므로 더 작은 에포크는 늦은 것
    q.onBuffer(makeBufferInfo({ epoch: 3, slot: 0 }), "dup-staged") // 이미 찬 슬롯
    q.onBuffer(makeBufferInfo({ epoch: 3, slot: 1, w: 640, h: 360 }), "odd-size") // 같은 뱅크인데 크기가 다르다
    // Assert
    expect(q.drainFreed()).toEqual(["dup-current", "late-e2", "dup-staged", "odd-size"])
  })
})

describe("FrameQueue — epoch swap (staging)", () => {
  it("T-ui-mpv-frames-07 [Business rule] the first frame of a complete new bank is a swap job; old buffers are freed only after it is drawn", () => {
    // Arrange
    const q = opened()
    withBank(q, 2, { w: 640, h: 360 })
    // Act
    q.onFrame(makeFrame({ epoch: 2, slot: 0, seq: 1, w: 640, h: 360 }))
    const job = q.takeDraw()
    // Assert
    expect(job).toMatchObject({ swap: true, buffer: "e2s0", frame: { epoch: 2 } })
    expect(q.drainFreed()).toEqual([]) // 아직 옛 버퍼를 쓰는 중
    if (job) q.drawn(job)
    expect(q.drainFreed().sort()).toEqual(["e1s0", "e1s1", "e1s2"])
    expect(q.currentEpoch).toBe(2)
    expect(q.drainReleases()).toEqual([{ epoch: 2, slot: 0, seq: 1 }])
  })

  it("T-ui-mpv-frames-08 [Boundary] a frame of a staging bank that is not complete yet is dropped and released, the old epoch keeps drawing", () => {
    // Arrange
    const q = opened()
    q.onBuffer(makeBufferInfo({ epoch: 2, slot: 0, w: 640, h: 360 }), "e2s0")
    q.onBuffer(makeBufferInfo({ epoch: 2, slot: 1, w: 640, h: 360 }), "e2s1")
    // Act
    q.onFrame(makeFrame({ epoch: 2, slot: 0, seq: 1, w: 640, h: 360 }))
    q.onFrame(makeFrame({ epoch: 1, slot: 1, seq: 7 }))
    // Assert
    expect(q.drainReleases()).toEqual([{ epoch: 2, slot: 0, seq: 1 }])
    expect(q.takeDraw()).toMatchObject({ swap: false, frame: { epoch: 1, seq: 7 } })
    expect(q.drainFreed()).toEqual([])
  })

  it("T-ui-mpv-frames-09 [Business rule] a staged frame wins over a pending old-epoch frame, which is discarded without a release at the swap", () => {
    // Arrange
    const q = opened()
    q.onFrame(makeFrame({ epoch: 1, slot: 0, seq: 2 })) // 옛 에포크 대기분
    withBank(q, 2)
    q.onFrame(makeFrame({ epoch: 2, slot: 1, seq: 1 }))
    // Act
    const job = q.takeDraw()
    if (job) q.drawn(job)
    // Assert
    expect(job?.swap).toBe(true)
    expect(q.takeDraw()).toBeNull()
    expect(q.drainReleases()).toEqual([{ epoch: 2, slot: 1, seq: 1 }])
  })

  it("T-ui-mpv-frames-10 [Business rule] a staging that never gets its first frame expires: staged buffers are freed, the old epoch stays current", () => {
    // Arrange
    const q = opened()
    withBank(q, 2)
    q.drainFreed()
    // Act
    const expired = q.expireStaging(2)
    q.onFrame(makeFrame({ epoch: 2, slot: 0, seq: 1 })) // 포기한 에포크의 늦은 프레임
    // Assert
    expect(expired).toBe(true)
    expect(q.drainFreed().sort()).toEqual(["e2s0", "e2s1", "e2s2"])
    expect(q.currentEpoch).toBe(1)
    expect(q.stagingEpoch).toBe(0)
    expect(q.drainReleases()).toEqual([{ epoch: 2, slot: 0, seq: 1 }])
    expect(q.takeDraw()).toBeNull()
    expect(q.expireStaging(2)).toBe(false)
  })

  it("T-ui-mpv-frames-11 [Concurrency] a newer staging epoch abandons the older one: its buffers are freed and its staged frame released", () => {
    // Arrange
    const q = opened()
    withBank(q, 2)
    q.onFrame(makeFrame({ epoch: 2, slot: 2, seq: 1 }))
    q.drainFreed()
    // Act
    withBank(q, 3)
    // Assert
    expect(q.drainFreed().sort()).toEqual(["e2s0", "e2s1", "e2s2"])
    expect(q.drainReleases()).toEqual([{ epoch: 2, slot: 2, seq: 1 }])
    expect(q.stagingEpoch).toBe(3)
    expect(q.expireStaging(2)).toBe(false) // 낡은 타이머는 새 스테이징을 건드리지 않는다
    expect(q.stagingEpoch).toBe(3)
  })
})

describe("FrameQueue — not rendering (hidden / render inactive)", () => {
  it("T-ui-mpv-frames-12 [Business rule] while not rendering every frame is released immediately and nothing is drawn", () => {
    // Arrange
    const q = opened()
    q.setRendering(false)
    // Act
    q.onFrame(makeFrame({ slot: 0, seq: 2 }))
    q.onFrame(makeFrame({ slot: 1, seq: 3 }))
    // Assert
    expect(q.takeDraw()).toBeNull()
    expect(q.hasPending()).toBe(false)
    expect(q.drainReleases()).toEqual([{ epoch: 1, slot: 0, seq: 2 }, { epoch: 1, slot: 1, seq: 3 }])
  })

  it("T-ui-mpv-frames-13 [Concurrency] turning rendering off releases the pending frame; a staged swap is committed without drawing", () => {
    // Arrange
    const q = opened()
    q.onFrame(makeFrame({ slot: 0, seq: 2 }))
    withBank(q, 2)
    q.onFrame(makeFrame({ epoch: 2, slot: 1, seq: 1 }))
    q.drainFreed()
    // Act
    q.setRendering(false)
    // Assert
    expect(q.currentEpoch).toBe(2)
    expect(q.drainFreed().sort()).toEqual(["e1s0", "e1s1", "e1s2"])
    expect(q.drainReleases()).toEqual([{ epoch: 1, slot: 0, seq: 2 }, { epoch: 2, slot: 1, seq: 1 }])
    // 다시 켜면 이전 상태 없이 새 프레임부터
    q.setRendering(true)
    expect(q.takeDraw()).toBeNull()
  })

  it("T-ui-mpv-frames-14 [Business rule] a staging frame arriving while not rendering commits the swap and releases the frame", () => {
    // Arrange
    const q = opened()
    q.setRendering(false)
    withBank(q, 2)
    q.drainFreed()
    // Act
    q.onFrame(makeFrame({ epoch: 2, slot: 0, seq: 1 }))
    // Assert
    expect(q.currentEpoch).toBe(2)
    expect(q.drainFreed()).toHaveLength(3)
    expect(q.drainReleases()).toEqual([{ epoch: 2, slot: 0, seq: 1 }])
  })
})

describe("FrameQueue — dispose and release grouping", () => {
  it("T-ui-mpv-frames-15 [Business rule] dispose frees every buffer of every bank, drops queued releases and ignores later input", () => {
    // Arrange
    const q = opened()
    withBank(q, 2)
    q.onFrame(makeFrame({ slot: 0, seq: 2 }))
    q.drainFreed()
    // Act
    q.dispose()
    q.onFrame(makeFrame({ slot: 1, seq: 3 }))
    q.onBuffer(makeBufferInfo({ epoch: 5, slot: 0 }), "after")
    // Assert
    expect(q.drainFreed().sort()).toEqual(["after", "e1s0", "e1s1", "e1s2", "e2s0", "e2s1", "e2s2"])
    expect(q.drainReleases()).toEqual([])
    expect(q.takeDraw()).toBeNull()
  })

  it("T-ui-mpv-frames-16 [Boundary] groupReleases makes one message per epoch, chunks of at most 8 slots, and drops exact duplicates", () => {
    // Arrange
    const many = Array.from({ length: 10 }, (_, i) => ({ epoch: 4, slot: i % 3, seq: i + 1 }))
    // Act
    const msgs = groupReleases([...many, { epoch: 4, slot: 0, seq: 1 }, { epoch: 5, slot: 2, seq: 1 }])
    // Assert
    expect(msgs.map((m) => [m.epoch, m.slots.length])).toEqual([[4, 8], [4, 2], [5, 1]])
    expect(groupReleases([])).toEqual([])
    for (const m of msgs) expect(m).toMatchObject({ mam: "player.v1", k: "release" })
  })
})
