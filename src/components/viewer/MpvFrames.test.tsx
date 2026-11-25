// T-ui-mpv-frames-20..: SharedBuffer 수신 → WebGL 업로드·그리기 → release (chrome.webview / WebGL 스텁, rAF 는 명시적 큐)
// Categories: Happy (-20 draw+release, -21 batching, -26 epoch swap) / External failure (-23 missing buffer, -24 malformed) / Boundary (-25 stale epoch, -27 staging timeout)
//             Business rule (-22 objects only, -28 unmount, -29 hidden, -30 collapsed stage, -32 frames before ready, -34 aspect) / Concurrency (-31 context lost)
import { act, screen } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import "@/i18n"
import { mountMpv } from "@/test/mpv"
import { makeBufferInfo, makeFrame } from "@/test/webview2"
import { RENDER_SIZE_DEBOUNCE_MS } from "./useCanvasSize"
import { STAGING_TIMEOUT_MS } from "./frameQueue"

const STRIDE_1000 = 4032 // 1000×4 = 4000 → 64 의 배수로 올림

beforeEach(() => {
  Element.prototype.hasPointerCapture ??= () => false
  Element.prototype.scrollIntoView ??= () => undefined
})
afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  Reflect.deleteProperty(document, "hidden")
})

const setHidden = (hidden: boolean) => {
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden })
  act(() => void document.dispatchEvent(new Event("visibilitychange")))
}

async function opened(over: Parameters<typeof mountMpv>[0] = {}) {
  const t = await mountMpv(over)
  await t.ready()
  return t
}

describe("MpvPlayer — frame upload and release", () => {
  it("T-ui-mpv-frames-20 [Happy] a frame is uploaded with the row length stride/4, drawn once, and released as a 'player.v1' object after the task", async () => {
    // Arrange
    const t = await opened()
    // Act
    t.sendFrame({ slot: 1, seq: 7 })
    // Assert: 업로드 — stride 에 맞춘 행 길이, 캔버스 속성 = 프레임 크기
    expect(t.gl.uploads).toEqual([{ w: 1000, h: 563, rowLength: STRIDE_1000 / 4, bytes: STRIDE_1000 * 563, canvas: { w: 1000, h: 563 } }])
    expect(t.gl.draws).toEqual([{ w: 1000, h: 563 }])
    expect(t.webview.releases()).toHaveLength(0) // microtask 전에는 아직
    await t.settle()
    expect(t.webview.releases()).toEqual([{ mam: "player.v1", k: "release", epoch: 1, slots: [{ slot: 1, seq: 7 }] }])
  })

  it("T-ui-mpv-frames-21 [Concurrency] frames arriving before a rAF supersede each other: only the newest is drawn, and all releases leave in one message", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 }) // 에포크 1 확정
    await t.settle()
    t.webview.posted.length = 0
    t.gl.uploads.length = 0
    // Act: rAF 전에 두 장
    act(() => {
      t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 1, seq: 2 }))
      t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 2, seq: 3 }))
    })
    t.flushRaf()
    await t.settle()
    // Assert
    expect(t.gl.uploads).toHaveLength(1)
    expect(t.webview.releases()).toHaveLength(1)
    expect(t.webview.releases()[0].slots).toEqual([{ slot: 1, seq: 2 }, { slot: 2, seq: 3 }])
  })

  it("T-ui-mpv-frames-22 [Business rule] regression: every message to the host is an object with the wire tag; window.ipc.postMessage is never used", async () => {
    // Arrange
    const t = await opened()
    // Act: 정상·비정상 입력을 섞어 release 가 여러 번 나가게 한다
    t.sendFrame({ slot: 0, seq: 1 })
    t.sendFrame({ slot: 1, seq: 2, stride: 4096 })
    act(() => t.webview.emitFrame("not an object"))
    await t.settle()
    // Assert
    expect(t.webview.posted.length).toBeGreaterThan(0)
    for (const m of t.webview.posted) {
      expect(typeof m).toBe("object")
      expect(m).toMatchObject({ mam: "player.v1", k: "release" })
    }
    expect(t.webview.ipcPostMessage).not.toHaveBeenCalled()
    expect(t.webview.chromePostMessage).not.toHaveBeenCalled()
  })

  it("T-ui-mpv-frames-36 [Business rule] regression: a drawn frame is returned through api.playerReleaseSlots (Tauri command), never chrome.webview.postMessage — the host never saw object messages, so every slot filled and video froze", async () => {
    // Arrange
    const t = await opened()
    // Act
    t.sendFrame({ slot: 2, seq: 5 })
    await t.settle()
    // Assert
    expect(t.releaseSlots).toHaveBeenCalledTimes(1)
    expect(t.releaseSlots).toHaveBeenCalledWith({ mam: "player.v1", k: "release", epoch: 1, slots: [{ slot: 2, seq: 5 }] })
    expect(t.webview.chromePostMessage).not.toHaveBeenCalled()
    expect(t.webview.ipcPostMessage).not.toHaveBeenCalled()
  })

  it("T-ui-mpv-frames-37 [External failure] a rejecting or throwing playerReleaseSlots is swallowed: no unhandled rejection, later frames are still drawn and released", async () => {
    // Arrange
    const t = await opened()
    t.releaseSlots.mockImplementationOnce(() => Promise.reject({ code: "internal", message: "x" }))
    t.releaseSlots.mockImplementationOnce(() => {
      throw new Error("sync failure")
    })
    // Act
    t.sendFrame({ slot: 0, seq: 1 })
    await t.settle()
    t.sendFrame({ slot: 1, seq: 2 })
    await t.settle()
    t.sendFrame({ slot: 2, seq: 3 })
    await t.settle()
    // Assert
    expect(t.gl.uploads).toHaveLength(3)
    expect(t.releaseSlots).toHaveBeenCalledTimes(3)
    expect(t.webview.releases()).toEqual([{ mam: "player.v1", k: "release", epoch: 1, slots: [{ slot: 2, seq: 3 }] }])
  })

  it("T-ui-mpv-frames-23 [External failure] a frame whose slot buffer never arrived is not drawn and is released; the canvas is untouched", async () => {
    // Arrange
    const t = await opened()
    act(() => void t.webview.emitBuffer(makeBufferInfo({ epoch: 1, slot: 0, w: 1000, h: 563 }))) // 슬롯 0 만 도착
    // Act
    act(() => t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 2, seq: 1 })))
    t.flushRaf()
    await t.settle()
    // Assert
    expect(t.gl.uploads).toHaveLength(0)
    expect(t.webview.releases()).toEqual([{ mam: "player.v1", k: "release", epoch: 1, slots: [{ slot: 2, seq: 1 }] }])
  })

  it("T-ui-mpv-frames-24 [External failure] malformed input: wrong stride / oversize frames are released undrawn, junk messages are ignored, bad or short buffers are freed at once", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    await t.settle()
    t.webview.posted.length = 0
    t.gl.uploads.length = 0
    // Act
    t.sendFrame({ slot: 1, seq: 2, stride: 4096 }) // stride 불일치
    t.sendFrame({ slot: 2, seq: 3, w: 1001, h: 563, stride: 4096 }) // 뱅크보다 넓다
    act(() => {
      t.webview.emitFrame(null)
      t.webview.emitFrame({ mam: "player.v1", k: "frame", epoch: "x" })
      t.webview.emitFrame({ mam: "other", k: "frame", epoch: 1, slot: 0, seq: 9, w: 1, h: 1, stride: 64, renderDoneMs: 1 })
    })
    const junk = t.webview.emitBuffer({ mam: "player.v1", epoch: 9, slot: 7 }, 16) // 모양이 틀린 info
    const short = t.webview.emitBuffer(makeBufferInfo({ epoch: 9, slot: 0, w: 1000, h: 563 }), 100) // info.size 보다 작은 버퍼
    await t.settle()
    // Assert
    expect(t.gl.uploads).toHaveLength(0)
    expect(t.webview.releases().flatMap((r) => r.slots.map((s) => s.seq)).sort()).toEqual([2, 3])
    expect(t.webview.freed).toEqual(expect.arrayContaining([junk, short]))
  })

  it("T-ui-mpv-frames-25 [Boundary] after an epoch swap a late frame of the old epoch is ignored without a release", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    act(() => t.webview.emitBank(2, { w: 1000, h: 563 }))
    act(() => t.webview.emitFrame(makeFrame({ epoch: 2, w: 1000, h: 563, slot: 0, seq: 1 })))
    t.flushRaf()
    await t.settle()
    t.webview.posted.length = 0
    t.gl.uploads.length = 0
    // Act
    act(() => t.webview.emitFrame(makeFrame({ epoch: 1, w: 1000, h: 563, slot: 1, seq: 9 })))
    t.flushRaf()
    await t.settle()
    // Assert
    expect(t.gl.uploads).toHaveLength(0)
    expect(t.webview.posted).toEqual([])
  })
})

describe("MpvPlayer — epoch swap", () => {
  it("T-ui-mpv-frames-26 [Happy] a size change swaps epochs without a black flash: the old size keeps drawing until the first new frame, which resizes and draws in one task, then the old buffers are freed", async () => {
    // Arrange
    const t = await opened()
    const first = t.webview.emitBank(1, { w: 1000, h: 563 }) // 에포크 1 (sendFrame 이 만들기 전에 직접)
    act(() => t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 0, seq: 1 })))
    t.flushRaf()
    const canvas = t.canvas()
    expect(canvas.width).toBe(1000)
    // Act 1: 새 에포크의 버퍼만 도착 — 캔버스 크기·옛 버퍼는 그대로
    const staged = t.webview.emitBank(2, { w: 640, h: 360 })
    expect(canvas.width).toBe(1000)
    expect(t.webview.freed).toEqual([])
    // Act 2: 첫 새 프레임
    t.gl.calls.length = 0
    act(() => t.webview.emitFrame(makeFrame({ epoch: 2, w: 640, h: 360, slot: 1, seq: 1 })))
    expect(canvas.width).toBe(1000) // 프레임 도착만으로는 바뀌지 않는다 (rAF 에서)
    t.flushRaf()
    // Assert: 같은 작업에서 크기 변경 → 텍스처 → 업로드 → 그리기, 그 뒤 옛 버퍼 해제
    expect(t.gl.calls).toEqual(["viewport:640x360", "texImage2D:640x360", "texSubImage2D:640x360", "draw"])
    expect(t.gl.draws[t.gl.draws.length - 1]).toEqual({ w: 640, h: 360 })
    expect(canvas.width).toBe(640)
    expect(t.webview.freed).toHaveLength(3)
    for (const b of t.webview.freed) expect(first).toContain(b)
    for (const b of staged) expect(t.webview.freed).not.toContain(b)
    await t.settle()
    expect(t.webview.releases()[t.webview.releases().length - 1]).toMatchObject({ epoch: 2, slots: [{ slot: 1, seq: 1 }] })
  })

  it("T-ui-mpv-frames-27 [Boundary] a staged epoch whose first frame never comes is abandoned after 2 s: staged buffers are freed, the old epoch keeps its buffers", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    const staged = t.webview.emitBank(2, { w: 640, h: 360 })
    // Act
    act(() => void vi.advanceTimersByTime(STAGING_TIMEOUT_MS - 1))
    expect(t.webview.freed).toEqual([])
    act(() => void vi.advanceTimersByTime(1))
    // Assert
    expect(t.webview.freed).toHaveLength(3)
    for (const b of staged) expect(t.webview.freed).toContain(b)
    // 포기한 에포크의 늦은 프레임은 그리지 않고 돌려준다
    t.gl.uploads.length = 0
    act(() => t.webview.emitFrame(makeFrame({ epoch: 2, w: 640, h: 360, slot: 0, seq: 1 })))
    t.flushRaf()
    expect(t.gl.uploads).toHaveLength(0)
  })
})

describe("MpvPlayer — staging timer", () => {
  it("T-ui-mpv-frames-35 [Concurrency] regression: frames of the old epoch arriving while an epoch is staged do not postpone the 2 s expiry", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    const staged = t.webview.emitBank(2, { w: 640, h: 360 })
    // Act: 1 초 뒤 옛 에포크의 프레임이 계속 온다
    act(() => void vi.advanceTimersByTime(1000))
    t.sendFrame({ slot: 1, seq: 2 })
    act(() => void vi.advanceTimersByTime(STAGING_TIMEOUT_MS - 1000 - 1))
    expect(t.webview.freed).toEqual([])
    act(() => void vi.advanceTimersByTime(1))
    // Assert: 첫 버퍼가 온 시점부터 2 초 — 프레임 도착과 무관하다
    expect(t.webview.freed).toHaveLength(3)
    for (const b of staged) expect(t.webview.freed).toContain(b)
  })
})

describe("MpvPlayer — render size stability", () => {
  it("T-ui-mpv-size-10 [Concurrency] regression: the render size comes from the asset aspect, not the frame size — a slightly narrower frame does not shrink it step by step (1366×750 stage, 16:9 video, DPR 1.25)", async () => {
    // Arrange
    vi.stubGlobal("devicePixelRatio", 1.25)
    const t = await mountMpv({ stageSize: { width: 1093.2, height: 599.6 }, detail: { width: 1280, height: 720 } })
    await t.ready()
    const opened = t.open.mock.calls[0][1]
    // Assert: 16:9 contain in 1366×750 device px = 1333×750 (±1)
    expect(Math.abs(opened.width - 1333)).toBeLessThanOrEqual(1)
    expect(opened.height).toBe(750)
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    // Act: 워커가 요청보다 좁은 프레임(1305×750)을 돌려주는 상황을 반복한다
    for (const w of [1324, 1314, 1305]) {
      act(() => {
        t.webview.emitBank(w, { w, h: 750 })
        t.webview.emitFrame(makeFrame({ epoch: w, w, h: 750, slot: 0, seq: 1 }))
      })
      t.flushRaf()
      act(() => void vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS * 2))
    }
    // Assert: 렌더 크기를 다시 보고하지 않는다 (프레임 크기를 비율로 쓰면 1324→1314→1305 로 줄어든다)
    expect(t.setRenderSize).not.toHaveBeenCalled()
  })
})

describe("MpvPlayer — lifecycle and gating", () => {
  it("T-ui-mpv-frames-28 [Business rule] unmounting frees every buffer once, removes the listeners, cancels the pending rAF and sends no further release", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    await t.settle()
    const before = t.webview.posted.length
    act(() => t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 1, seq: 2 }))) // 그리지 않은 대기 프레임
    expect(t.raf.pending()).toBeGreaterThan(0)
    // Act
    t.view.unmount()
    await t.settle()
    // Assert
    expect(t.webview.freed).toHaveLength(3)
    expect(new Set(t.webview.freed).size).toBe(3)
    expect(t.webview.listenerCount("sharedbufferreceived")).toBe(0)
    expect(t.webview.listenerCount("message")).toBe(0)
    expect(t.raf.pending()).toBe(0)
    expect(t.webview.posted).toHaveLength(before)
  })

  it("T-ui-mpv-frames-29 [Business rule] render-inactive (document hidden): the host is told, frames are released immediately without drawing or relying on rAF, and drawing resumes when visible", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    await t.settle()
    t.setRenderActive.mockClear()
    t.webview.posted.length = 0
    t.gl.uploads.length = 0
    // Act 1: 숨김
    setHidden(true)
    expect(t.setRenderActive).toHaveBeenLastCalledWith(false)
    act(() => {
      t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 1, seq: 2 }))
      t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 2, seq: 3 }))
    })
    await t.settle() // rAF 를 한 번도 실행하지 않았다
    // Assert 1
    expect(t.gl.uploads).toHaveLength(0)
    expect(t.webview.releases().flatMap((r) => r.slots.map((s) => s.seq))).toEqual([2, 3])
    // Act 2: 다시 보임
    setHidden(false)
    expect(t.setRenderActive).toHaveBeenLastCalledWith(true)
    t.sendFrame({ slot: 0, seq: 4 })
    // Assert 2
    expect(t.gl.uploads).toHaveLength(1)
  })

  it("T-ui-mpv-frames-30 [Business rule] a collapsed (0×0) viewer stage counts as render-inactive too", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    await t.settle()
    t.setRenderActive.mockClear()
    t.gl.uploads.length = 0
    // Act
    vi.mocked(Element.prototype.getBoundingClientRect).mockReturnValue({ x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) })
    act(() => void window.dispatchEvent(new Event("resize")))
    act(() => t.webview.emitFrame(makeFrame({ w: 1000, h: 563, slot: 1, seq: 2 })))
    await t.settle()
    // Assert
    expect(t.setRenderActive).toHaveBeenLastCalledWith(false)
    expect(t.gl.uploads).toHaveLength(0)
    expect(t.webview.releases()[t.webview.releases().length - 1]?.slots).toEqual([{ slot: 1, seq: 2 }])
  })

  it("T-ui-mpv-frames-31 [Concurrency] a lost WebGL context stops drawing but still returns slots; when restored the host is asked to re-render and drawing resumes", async () => {
    // Arrange
    const t = await opened()
    t.sendFrame({ slot: 0, seq: 1 })
    await t.settle()
    t.webview.posted.length = 0
    t.gl.uploads.length = 0
    t.setRenderActive.mockClear()
    // Act 1: 컨텍스트 소실
    const lost = t.gl.loseContext(t.canvas())
    t.sendFrame({ slot: 1, seq: 2 })
    await t.settle()
    // Assert 1
    expect(lost.defaultPrevented).toBe(true)
    expect(t.gl.uploads).toHaveLength(0)
    expect(t.webview.releases().flatMap((r) => r.slots.map((s) => s.seq))).toContain(2)
    // Act 2: 복구 — 지워진 마지막 프레임을 호스트가 다시 그리게 한다
    act(() => t.gl.restoreContext(t.canvas()))
    expect(t.setRenderActive).toHaveBeenCalledWith(true)
    t.sendFrame({ slot: 2, seq: 3 })
    // Assert 2
    expect(t.gl.uploads).toHaveLength(1)
  })

  it("T-ui-mpv-frames-32 [Business rule] the first frame arriving before player_open resolves is drawn (the host publishes it before the reply)", async () => {
    // Arrange
    const t = await mountMpv({ open: "pending" })
    await screen.findByRole("img", { name: "v1.mxf" })
    // Act
    t.sendFrame({ slot: 0, seq: 1 })
    // Assert
    expect(t.gl.uploads).toHaveLength(1)
    expect(t.stage()).toHaveAttribute("aria-busy", "true")
    await t.finishOpen()
    await t.ready()
  })

  it("T-ui-mpv-frames-33 [Business rule] both chrome.webview listeners are already installed at the moment player_open is called", async () => {
    // Arrange
    const seen: number[] = []
    // Act
    const t = await mountMpv({ onOpen: ({ webview }) => void seen.push(webview.listenerCount("sharedbufferreceived"), webview.listenerCount("message")) })
    await t.ready()
    // Assert
    expect(seen).toEqual([1, 1])
  })

  it("T-ui-mpv-gl-02 [Happy] the WebGL2 context is opaque and the fragment shader swizzles bgr0 to rgb with alpha forced to 1", async () => {
    // Arrange + Act
    const t = await opened()
    // Assert
    expect(t.gl.contextOptions[0]).toMatchObject({ alpha: false, antialias: false, depth: false, stencil: false })
    const fragment = t.gl.sources.find((s) => s.includes("sampler2D"))
    expect(fragment).toContain("vec4(t.b, t.g, t.r, 1.0)")
    expect(t.gl.sources.find((s) => s.includes("gl_VertexID"))).toBeDefined()
  })

  it("T-ui-mpv-frames-34 [Business rule] the first frame's aspect drives the stage: a square video re-measures the contained size and reports it after the debounce", async () => {
    // Arrange: 영상 정보(폭·높이)가 없어 16:9 로 열린다
    const t = await opened({ detail: { width: null, height: null } })
    expect(t.open).toHaveBeenCalledWith(1, { width: 1000, height: 563 }, true)
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] })
    // Act: 첫 프레임이 정사각형 (600×600)
    act(() => {
      t.webview.emitBank(1, { w: 600, h: 600 })
      t.webview.emitFrame(makeFrame({ w: 600, h: 600, slot: 0, seq: 1 }))
    })
    t.flushRaf()
    act(() => void vi.advanceTimersByTime(RENDER_SIZE_DEBOUNCE_MS))
    // Assert: 무대 1000×600 에 1:1 contain → 600×600
    expect(t.setRenderSize).toHaveBeenCalledWith(600, 600)
  })
})
