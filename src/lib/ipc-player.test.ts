// Categories: Happy (command 이름·인자 모양) / Validation (event 계약·오류 코드) / External (reject 전파·mock 폴백)
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import { errorKey, isCommandError } from "@/lib/errors"
import { PLAYER_RETRYABLE } from "@/lib/ipc-contract-player"
import type { ReleaseMsg } from "@/lib/ipc-player-wire"
import {
  onPlayerState,
  playerAvailable,
  playerClose,
  playerControl,
  playerOpen,
  playerReleaseSlots,
  playerRestart,
  playerSeek,
  playerSetRate,
  playerSetRenderActive,
  playerSetRenderSize,
  playerSetVolume,
  systemVolumeStep,
  PLAYER_STATE_EVENT,
  type CanvasSize,
  type PlayerStatePayload,
} from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

const CANVAS: CanvasSize = { width: 800, height: 450 }
const OPEN_REPLY = { sessionId: 4, lengthMs: 83_000, fps: 25, state: "paused", timeMs: 0 } as const

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockReset()
})

describe("ipc player commands", () => {
  it("T-ui-ipc-player-01 [Happy] player_available takes no args and returns the boolean", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(true)
    // Act
    const result = await playerAvailable()
    // Assert
    expect(invoke).toHaveBeenCalledWith("player_available")
    expect(result).toBe(true)
  })

  it("T-ui-ipc-player-02 [Happy] player_open sends { assetId, autoplay, canvas } and returns the open reply (sessionId, state, time)", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(OPEN_REPLY)
    // Act
    const result = await playerOpen(7, CANVAS, false)
    // Assert
    expect(invoke).toHaveBeenCalledWith("player_open", { assetId: 7, autoplay: false, canvas: CANVAS })
    expect(result).toEqual(OPEN_REPLY)
  })

  it("T-ui-ipc-player-03 [External] player_open propagates the structured error codes", async () => {
    // Arrange
    for (const code of ["player_unavailable", "asset_offline", "open_failed", "player_timeout"]) {
      vi.mocked(invoke).mockRejectedValueOnce({ code, message: code })
      // Act + Assert
      await expect(playerOpen(1, CANVAS, true)).rejects.toEqual({ code, message: code })
    }
  })

  it("T-ui-ipc-player-04 [Happy] seek, set_rate, set_render_size, set_render_active use camelCase arg objects", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await playerSeek(14_500)
    await playerSetRate(1.5)
    await playerSetRenderSize(1280, 720)
    await playerSetRenderActive(false)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["player_seek", { ms: 14_500 }],
      ["player_set_rate", { rate: 1.5 }],
      ["player_set_render_size", { width: 1280, height: 720 }],
      ["player_set_render_active", { active: false }],
    ])
  })

  it("T-ui-ipc-player-05 [Happy] player_set_volume sends { volume, muted } and player_close takes no args", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await playerSetVolume(35, true)
    await playerClose()
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([["player_set_volume", { volume: 35, muted: true }], ["player_close"]])
  })

  it("T-ui-ipc-player-06 [Boundary] player_control sends { action } alone — there is no fps argument any more", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await playerControl("toggle")
    await playerControl("stepBack")
    // Assert
    expect(vi.mocked(invoke).mock.calls[0]).toEqual(["player_control", { action: "toggle" }])
    expect(Object.keys(vi.mocked(invoke).mock.calls[0][1] as object)).toEqual(["action"])
    expect(vi.mocked(invoke).mock.calls[1]).toEqual(["player_control", { action: "stepBack" }])
  })

  it("T-ui-ipc-player-mpv-01 [Happy] player_restart takes no args and returns the open reply; a failure keeps its code", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce({ ...OPEN_REPLY, sessionId: 9, state: "playing" })
    vi.mocked(invoke).mockRejectedValueOnce({ code: "player_restart_failed", message: "x" })
    // Act + Assert
    await expect(playerRestart()).resolves.toMatchObject({ sessionId: 9, state: "playing" })
    expect(invoke).toHaveBeenCalledWith("player_restart")
    await expect(playerRestart()).rejects.toEqual({ code: "player_restart_failed", message: "x" })
  })

  it("T-ui-ipc-player-mpv-02 [Validation] the Tauri api exposes the new commands and none of the removed ones", async () => {
    // Arrange
    const api = createTauriApi() as unknown as Record<string, unknown>
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await (api.playerSetRenderSize as (w: number, h: number) => Promise<void>)(640, 360)
    // Assert
    expect(vi.mocked(invoke)).toHaveBeenCalledWith("player_set_render_size", { width: 640, height: 360 })
    for (const removed of ["playerSetRect", "playerSetVisible", "playerSnapshot", "onPlayerInput"]) expect(api[removed]).toBeUndefined()
    for (const added of ["playerSetRenderSize", "playerSetRenderActive", "playerRestart"]) expect(typeof api[added]).toBe("function")
  })

  it("T-ui-ipc-player-mpv-04 [Happy/External] player_release_slots sends { release } as is; the Tauri api rejects with a CommandError and the mock is a no-op", async () => {
    // Arrange
    const release: ReleaseMsg = { mam: "player.v1", k: "release", epoch: 3, slots: [{ slot: 0, seq: 4 }] }
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockRejectedValueOnce({ code: "internal", message: "x" })
    const api = createTauriApi()
    // Act + Assert
    await expect(playerReleaseSlots(release)).resolves.toBeUndefined()
    expect(invoke).toHaveBeenCalledWith("player_release_slots", { release })
    await expect(api.playerReleaseSlots(release)).rejects.toEqual({ code: "internal", message: "x" })
    await expect(createMockApi({ count: 5 }).playerReleaseSlots(release)).resolves.toBeUndefined()
  })

  it("T-ui-ipc-player-mpv-03[Validation] the new error codes are known (own message key); only timeout and restarting are retryable", () => {
    // Arrange + Act + Assert
    for (const code of ["player_timeout", "player_restarting", "player_restart_failed", "player_worker_crashed"]) {
      expect(isCommandError({ code, message: "x" })).toBe(true)
      expect(errorKey({ code, message: "x" })).toBe(`errors.${code}`)
    }
    expect([...PLAYER_RETRYABLE].sort()).toEqual(["player_restarting", "player_timeout"])
  })

  it("T-ui-ipc-player-08 [Validation] onPlayerState listens to 'player://state', unwraps the payload and returns the unlisten", async () => {
    // Arrange
    const unlisten = vi.fn()
    let registered: ((e: { payload: PlayerStatePayload }) => void) | undefined
    vi.mocked(listen).mockImplementation(async (_name, cb) => {
      registered = cb as typeof registered
      return unlisten
    })
    const handler = vi.fn()
    const payload: PlayerStatePayload = { sessionId: 2, assetId: 3, state: "playing", timeMs: 1200, lengthMs: 9000, fps: 25 }
    // Act
    const off = await onPlayerState(handler)
    registered?.({ payload })
    // Assert
    expect(PLAYER_STATE_EVENT).toBe("player://state")
    expect(vi.mocked(listen).mock.calls[0][0]).toBe("player://state")
    expect(handler).toHaveBeenCalledWith(payload)
    expect(off).toBe(unlisten)
  })
})

describe("mock api player", () => {
  it("T-ui-ipc-player-09 [External] the browser mock reports the player as unavailable; open and restart reject with player_unavailable", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act + Assert
    await expect(api.playerAvailable()).resolves.toBe(false)
    await expect(api.playerOpen(1, CANVAS, false)).rejects.toMatchObject({ code: "player_unavailable" })
    await expect(api.playerRestart()).rejects.toMatchObject({ code: "player_unavailable" })
    await expect(api.playerSetRenderSize(10, 10)).resolves.toBeUndefined()
    await expect(api.playerSetRenderActive(false)).resolves.toBeUndefined()
    const off = await api.onPlayerState(() => undefined)
    expect(() => off()).not.toThrow()
  })
})

describe("system volume contract", () => {
  it("T-ui-ipc-sysvol-01 [Happy] system_volume_step sends { delta } and returns the new 0..1 volume", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(0.65)
    // Act
    const volume = await systemVolumeStep(-0.05)
    // Assert
    expect(invoke).toHaveBeenCalledWith("system_volume_step", { delta: -0.05 })
    expect(volume).toBe(0.65)
  })

  it("T-ui-ipc-sysvol-02 [External] audio_device_unavailable rejects through the Tauri api, is a known error code with its own message key", async () => {
    // Arrange
    const err = { code: "audio_device_unavailable", message: "no output device" }
    vi.mocked(invoke).mockRejectedValue(err)
    const api = createTauriApi()
    // Act + Assert
    await expect(api.systemVolumeStep(0.05)).rejects.toMatchObject({ code: "audio_device_unavailable" })
    expect(isCommandError(err)).toBe(true)
    expect(errorKey(err)).toBe("errors.audio_device_unavailable")
  })

  it("T-ui-ipc-sysvol-03 [External] the browser mock has no system volume: it rejects with audio_device_unavailable (never succeeds silently)", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act + Assert
    await expect(api.systemVolumeStep(0.05)).rejects.toMatchObject({ code: "audio_device_unavailable" })
  })
})
