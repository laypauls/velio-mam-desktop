// Categories: Happy (T-ui-ipc-vol-01a, 01b) / External (01c) / Business rule (01d remember, 01e unignore) / Validation (01f path helper)
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import type { IgnoredVolume, PendingVolume } from "@/lib/ipc-contract"
import {
  PENDING_VOLUMES_EVENT,
  dismissNewVolume,
  listIgnoredVolumes,
  listPendingNewVolumes,
  pickDirectory,
  onPendingVolumesChanged,
  unignoreVolume,
} from "./ipc"
import { isInsideMount } from "./volumes"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

/** Rust 가 돌려주는 모양(camelCase) */
const USB: PendingVolume = { volumeKey: "vol-usb", label: "SanDisk USB", kind: "removable", mountPoint: "E:\\", totalBytes: 62_500_000_000 }
const IGN: IgnoredVolume = { volumeKey: "vol-cam", label: null, lastMountPoint: "G:\\", ignoredAt: 1_700_000_000_000 }

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockReset()
})

describe("ipc volume commands", () => {
  it("T-ui-ipc-vol-01a [Happy] wrappers use the Rust command and argument names and return the server shapes", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce([USB]).mockResolvedValueOnce(undefined).mockResolvedValueOnce([IGN]).mockResolvedValueOnce(undefined)
    // Act
    const pending = await listPendingNewVolumes()
    await dismissNewVolume("vol-usb", true)
    const ignored = await listIgnoredVolumes()
    await unignoreVolume("vol-cam")
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["list_pending_new_volumes"],
      ["dismiss_new_volume", { volumeKey: "vol-usb", remember: true }],
      ["list_ignored_volumes"],
      ["unignore_volume", { volumeKey: "vol-cam" }],
    ])
    expect([pending, ignored]).toEqual([[USB], [IGN]])
  })

  it("T-ui-ipc-vol-01b [Happy] volumes://pending-changed delivers the full list and unlistens", async () => {
    // Arrange
    const unlisten = vi.fn()
    let registered: ((e: { payload: { volumes: PendingVolume[] } }) => void) | undefined
    vi.mocked(listen).mockImplementation(async (_name, cb) => {
      registered = cb as typeof registered
      return unlisten
    })
    const handler = vi.fn()
    // Act
    const off = await onPendingVolumesChanged(handler)
    registered?.({ payload: { volumes: [USB] } })
    registered?.({ payload: { volumes: [] } })
    // Assert
    expect(PENDING_VOLUMES_EVENT).toBe("volumes://pending-changed")
    expect(vi.mocked(listen).mock.calls[0][0]).toBe("volumes://pending-changed")
    expect(handler.mock.calls).toEqual([[[USB]], [[]]])
    expect(off).toBe(unlisten)
  })

  it("T-ui-ipc-vol-01c [External] the Tauri api passes every volume command through and keeps structured errors", async () => {
    // Arrange
    const api = createTauriApi()
    vi.mocked(invoke).mockResolvedValue([])
    // Act
    await api.listPendingNewVolumes()
    await api.dismissNewVolume("k", false)
    await api.listIgnoredVolumes()
    await api.unignoreVolume("k")
    // Assert
    expect(vi.mocked(invoke).mock.calls.map((c) => c[0])).toEqual(["list_pending_new_volumes", "dismiss_new_volume", "list_ignored_volumes", "unignore_volume"])
    vi.mocked(invoke).mockRejectedValueOnce({ code: "not_found", message: "volume" })
    await expect(api.dismissNewVolume("gone", true)).rejects.toMatchObject({ code: "not_found" })
  })
})

describe("pickDirectory defaultPath", () => {
  it("T-ui-ipc-vol-01g [Happy] sends no args when defaultPath is omitted and { defaultPath } when given", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(null)
    // Act
    await pickDirectory()
    await pickDirectory("E:\\")
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([["pick_directory"], ["pick_directory", { defaultPath: "E:\\" }]])
  })
})

describe("mock volume server rules", () => {
  it("T-ui-ipc-vol-01d [Business rule] dismiss: remember saves to the ignored list, not remembering only drops it; both emit the full pending list; unknown key rejected", async () => {
    // Arrange
    const other: PendingVolume = { ...USB, volumeKey: "vol-bak", label: "Backup", mountPoint: "F:\\" }
    const api = createMockApi({ count: 5, volumes: { pending: [USB, other] }, now: () => 42 })
    const events: PendingVolume[][] = []
    await api.onPendingVolumesChanged((v) => events.push(v))
    // Act
    await api.dismissNewVolume("vol-usb", true)
    await api.dismissNewVolume("vol-bak", false)
    // Assert
    expect(events.map((e) => e.map((v) => v.volumeKey))).toEqual([["vol-bak"], []])
    await expect(api.listIgnoredVolumes()).resolves.toEqual([{ volumeKey: "vol-usb", label: "SanDisk USB", lastMountPoint: "E:\\", ignoredAt: 42 }])
    await expect(api.listPendingNewVolumes()).resolves.toEqual([])
    await expect(api.dismissNewVolume("vol-usb", true)).rejects.toMatchObject({ code: "not_found" })
  })

  it("T-ui-ipc-vol-01e [Business rule] unignore removes the row and re-queues a connected disk; unknown key rejected", async () => {
    // Arrange
    const api = createMockApi({ count: 5, volumes: { ignored: [IGN], connected: [{ ...USB, volumeKey: "vol-cam", mountPoint: "G:\\" }] } })
    const events: PendingVolume[][] = []
    await api.onPendingVolumesChanged((v) => events.push(v))
    // Act
    await api.unignoreVolume("vol-cam")
    // Assert
    await expect(api.listIgnoredVolumes()).resolves.toEqual([])
    expect(events.map((e) => e.map((v) => v.volumeKey))).toEqual([["vol-cam"]])
    await expect(api.unignoreVolume("vol-cam")).rejects.toMatchObject({ code: "not_found" })
  })
})

describe("isInsideMount", () => {
  it("T-ui-ipc-vol-01f [Validation] case-insensitive, slash tolerant, rejects siblings sharing a prefix and other drives", () => {
    // Arrange + Act + Assert
    expect(isInsideMount("E:\\Media", "E:\\")).toBe(true)
    expect(isInsideMount("e:/media/sub", "E:\\")).toBe(true)
    expect(isInsideMount("E:\\", "E:\\")).toBe(true)
    expect(isInsideMount("F:\\Media", "E:\\")).toBe(false)
    expect(isInsideMount("D:\\Media", "D:\\Med")).toBe(false)
    expect(isInsideMount("D:\\Med\\x", "D:\\Med")).toBe(true)
  })
})
