// Categories: Happy (golden fixture parity) / Validation (guards reject malformed wire values) / Boundary (stride·size·slot limits)
// golden fixture `src/test/fixtures/player-wire.json` 는 Rust(cargo test)도 읽는다 — 한쪽만 바꾸면 이 테스트가 깨진다.
import { describe, expect, it } from "vitest"
import fixture from "@/test/fixtures/player-wire.json"
import { makeBufferInfo, makeFrame } from "@/test/webview2"
import { PLAYER_RETRYABLE, type PlayerErrorCode, type PlayerOpenResult, type PlayerStateName, type PlayerStatePayload } from "./ipc-contract-player"
import { isBufferInfo, isFrameMsg, isReleaseMsg, PLAYER_WIRE, type BufferInfo, type FrameMsg, type ReleaseMsg } from "./ipc-player-wire"

// 타입의 키 전체 — 키를 더하거나 빼면 `satisfies` 가 컴파일 오류를 낸다 (한쪽만 바뀐 계약을 잡는다)
const BUFFER_KEYS = { mam: 1, epoch: 1, slot: 1, count: 1, size: 1, w: 1, h: 1, stride: 1, fmt: 1 } satisfies Record<keyof BufferInfo, 1>
const FRAME_KEYS = { mam: 1, k: 1, epoch: 1, slot: 1, seq: 1, w: 1, h: 1, stride: 1, renderDoneMs: 1 } satisfies Record<keyof FrameMsg, 1>
const RELEASE_KEYS = { mam: 1, k: 1, epoch: 1, slots: 1 } satisfies Record<keyof ReleaseMsg, 1>
const OPEN_KEYS = { sessionId: 1, lengthMs: 1, fps: 1, state: 1, timeMs: 1 } satisfies Record<keyof PlayerOpenResult, 1>
const STATE_REQUIRED = { sessionId: 1, assetId: 1, state: 1, timeMs: 1, lengthMs: 1, fps: 1 } satisfies Partial<Record<keyof PlayerStatePayload, 1>>
const STATE_OPTIONAL = { buffering: 1, error: 1, errorCode: 1, restart: 1 } satisfies Partial<Record<keyof PlayerStatePayload, 1>>
const STATE_NAMES: Record<PlayerStateName, 1> = { opening: 1, playing: 1, paused: 1, ended: 1, error: 1, restarting: 1, restartFailed: 1 }
const ERROR_CODES: Record<PlayerErrorCode, 1> = {
  player_unavailable: 1, asset_offline: 1, open_failed: 1, audio_device_unavailable: 1, player_timeout: 1, player_restarting: 1, player_restart_failed: 1, player_worker_crashed: 1,
}

const keys = (o: object) => Object.keys(o).sort()

describe("player wire golden fixture", () => {
  it("T-ui-wire-01 [Happy] the fixture's BufferInfo / FrameMsg / ReleaseMsg pass the guards and carry exactly the typed keys", () => {
    // Arrange + Act + Assert
    expect(isBufferInfo(fixture.bufferInfo)).toBe(true)
    expect(isFrameMsg(fixture.frameMsg)).toBe(true)
    expect(isReleaseMsg(fixture.releaseMsg)).toBe(true)
    expect(keys(fixture.bufferInfo)).toEqual(keys(BUFFER_KEYS))
    expect(keys(fixture.frameMsg)).toEqual(keys(FRAME_KEYS))
    expect(keys(fixture.releaseMsg)).toEqual(keys(RELEASE_KEYS))
    expect(fixture.bufferInfo.mam).toBe(PLAYER_WIRE)
    expect(fixture.frameMsg.stride).toBe(fixture.bufferInfo.stride)
  })

  it("T-ui-wire-02 [Happy] the fixture's open reply has exactly the typed keys and a playing/paused state", () => {
    // Arrange + Act + Assert
    expect(keys(fixture.openReply)).toEqual(keys(OPEN_KEYS))
    expect(["playing", "paused"]).toContain(fixture.openReply.state)
  })

  it("T-ui-wire-03 [Happy] every fixture state event has the required keys, only known optional keys, a known state and error code", () => {
    // Arrange
    const events = Object.entries(fixture.stateEvents)
    // Act + Assert
    expect(events.length).toBeGreaterThanOrEqual(6)
    for (const [name, ev] of events) {
      const got = keys(ev)
      for (const required of Object.keys(STATE_REQUIRED)) expect(got, `${name} has ${required}`).toContain(required)
      for (const k of got) expect({ ...STATE_REQUIRED, ...STATE_OPTIONAL }, `${name}: unknown key ${k}`).toHaveProperty(k)
      expect(STATE_NAMES, `${name} state`).toHaveProperty((ev as { state: string }).state)
      const code = (ev as { errorCode?: string }).errorCode
      if (code !== undefined) expect(ERROR_CODES, `${name} errorCode`).toHaveProperty(code)
    }
    // 재시작 이벤트는 시도 횟수를 싣는다
    expect(fixture.stateEvents.restarting.restart).toEqual({ attempt: 1, max: 3 })
    expect(fixture.stateEvents.restartFailed.errorCode).toBe("player_worker_crashed")
    expect(PLAYER_RETRYABLE.has("player_worker_crashed")).toBe(false)
  })

  it("T-ui-wire-04 [Validation] every invalid release message of the fixture is rejected by the release guard", () => {
    // Arrange + Act + Assert
    expect(fixture.invalidReleaseMessages.length).toBeGreaterThanOrEqual(5)
    for (const msg of fixture.invalidReleaseMessages) expect(isReleaseMsg(msg)).toBe(false)
  })
})

describe("player wire guards", () => {
  it("T-ui-wire-05 [Validation] isBufferInfo rejects a wrong tag, count, format, slot, epoch and non-objects", () => {
    // Arrange
    const ok = makeBufferInfo()
    const bad: unknown[] = [
      null, "x", 1, [], { ...ok, mam: "other" }, { ...ok, count: 2 }, { ...ok, fmt: "rgba" }, { ...ok, slot: 3 }, { ...ok, slot: -1 }, { ...ok, epoch: 0 },
      { ...ok, epoch: 1.5 }, { ...ok, w: 0 }, { ...ok, h: 99999 }, { ...ok, w: "1280" },
    ]
    // Act + Assert
    expect(isBufferInfo(ok)).toBe(true)
    for (const v of bad) expect(isBufferInfo(v)).toBe(false)
  })

  it("T-ui-wire-06 [Boundary] isBufferInfo requires stride to be a multiple of 64 that holds a row, and size to hold every row", () => {
    // Arrange + Act + Assert
    expect(isBufferInfo(makeBufferInfo({ w: 16, h: 4, stride: 64, size: 256 }))).toBe(true)
    expect(isBufferInfo(makeBufferInfo({ w: 16, h: 4, stride: 63, size: 252 }))).toBe(false) // 64 의 배수 아님
    expect(isBufferInfo(makeBufferInfo({ w: 32, h: 4, stride: 64, size: 256 }))).toBe(false) // stride < w*4
    expect(isBufferInfo(makeBufferInfo({ w: 16, h: 4, stride: 64, size: 255 }))).toBe(false) // size < stride*h
  })

  it("T-ui-wire-07 [Validation] isFrameMsg rejects a wrong tag/kind, slot out of range, seq 0, bad numbers and a release message", () => {
    // Arrange
    const ok = makeFrame()
    const bad: unknown[] = [
      null, undefined, "frame", { ...ok, mam: "x" }, { ...ok, k: "release" }, { ...ok, slot: 3 }, { ...ok, seq: 0 }, { ...ok, epoch: 0 }, { ...ok, w: 0 },
      { ...ok, stride: 2 }, { ...ok, renderDoneMs: Number.NaN }, { ...ok, renderDoneMs: "1" }, fixture.releaseMsg,
    ]
    // Act + Assert
    expect(isFrameMsg(ok)).toBe(true)
    for (const v of bad) expect(isFrameMsg(v)).toBe(false)
  })

  it("T-ui-wire-08 [Boundary] isReleaseMsg accepts 1..8 slots and rejects 0 or 9", () => {
    // Arrange
    const slots = (n: number) => Array.from({ length: n }, (_, i) => ({ slot: i % 3, seq: i + 1 }))
    const msg = (n: number) => ({ mam: PLAYER_WIRE, k: "release", epoch: 2, slots: slots(n) })
    // Act + Assert
    expect(isReleaseMsg(msg(1))).toBe(true)
    expect(isReleaseMsg(msg(8))).toBe(true)
    expect(isReleaseMsg(msg(0))).toBe(false)
    expect(isReleaseMsg(msg(9))).toBe(false)
  })
})
