// Categories: Happy (T-ui-ipc-vcl-01, 02) / External (03 error codes) / Business rule (04 mock forget) / Validation (05 error mapping)
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import { errorKey, isCommandError } from "@/lib/errors"
import type { FolderRemovalImpact, FolderRemovalProgress, KnownVolume } from "@/lib/ipc-contract"
import { FOLDER_REMOVAL_EVENT, folderRemovalImpact, forgetVolumeRecord, listKnownVolumes, onFolderRemovalProgress } from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

/** Rust 가 돌려주는 모양(camelCase) */
const IMPACT: FolderRemovalImpact = { mediaCount: 1204, tagLinks: 120, notes: 8, collectionItems: 34, aiResults: 980 }
const CAMERA: KnownVolume = {
  volumeKey: "vol-cam", label: "CAMERA", kind: "removable", lastMountPoint: "G:\\", availability: "offline",
  lastSeenAt: 1_700_000_000_000, folderCount: 0, ignored: false,
}
const OLD: KnownVolume = {
  volumeKey: "vol-old", label: null, kind: null, lastMountPoint: "H:\\", availability: null, lastSeenAt: null, folderCount: 0, ignored: true,
}

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockReset()
})

describe("folder removal / known volume ipc", () => {
  it("T-ui-ipc-vcl-01 [Happy] wrappers use the Rust command and argument names and return the server shapes", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce(IMPACT).mockResolvedValueOnce([CAMERA, OLD]).mockResolvedValueOnce(undefined)
    // Act
    const impact = await folderRemovalImpact(3)
    const known = await listKnownVolumes()
    await forgetVolumeRecord("vol-cam")
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["folder_removal_impact", { folderId: 3 }],
      ["list_known_volumes"],
      ["forget_volume_record", { volumeKey: "vol-cam" }],
    ])
    expect(impact).toEqual(IMPACT)
    expect(known).toEqual([CAMERA, OLD])
  })

  it("T-ui-ipc-vcl-02 [Happy] folders://removal-progress delivers each payload and unlistens", async () => {
    // Arrange
    const unlisten = vi.fn()
    let registered: ((e: { payload: FolderRemovalProgress }) => void) | undefined
    vi.mocked(listen).mockImplementation(async (_name, cb) => {
      registered = cb as typeof registered
      return unlisten
    })
    const handler = vi.fn()
    const progress: FolderRemovalProgress = { folderId: 3, done: 500, total: 1204, finished: false, error: null }
    // Act
    const off = await onFolderRemovalProgress(handler)
    registered?.({ payload: progress })
    // Assert
    expect(FOLDER_REMOVAL_EVENT).toBe("folders://removal-progress")
    expect(vi.mocked(listen).mock.calls[0][0]).toBe("folders://removal-progress")
    expect(handler.mock.calls).toEqual([[progress]])
    expect(off).toBe(unlisten)
  })

  it("T-ui-ipc-vcl-03 [External] the Tauri api passes the new commands through and keeps backup_failed / volume_has_folders / busy", async () => {
    // Arrange
    const api = createTauriApi()
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await api.folderRemovalImpact(1)
    await api.listKnownVolumes()
    await api.forgetVolumeRecord("k")
    await api.removeFolder(1)
    // Assert
    expect(vi.mocked(invoke).mock.calls.map((c) => c[0])).toEqual(["folder_removal_impact", "list_known_volumes", "forget_volume_record", "remove_folder"])
    for (const code of ["backup_failed", "busy"]) {
      vi.mocked(invoke).mockRejectedValueOnce({ code, message: "x" })
      await expect(api.removeFolder(1)).rejects.toMatchObject({ code })
    }
    vi.mocked(invoke).mockRejectedValueOnce({ code: "volume_has_folders", message: "x" })
    await expect(api.forgetVolumeRecord("k")).rejects.toMatchObject({ code: "volume_has_folders" })
  })

  it("T-ui-ipc-vcl-04 [Validation] the new error codes are known and map to their own translation keys", () => {
    // Arrange + Act + Assert
    for (const code of ["backup_failed", "volume_has_folders", "busy"]) {
      expect(isCommandError({ code, message: "x" })).toBe(true)
      expect(errorKey({ code, message: "x" })).toBe(`errors.${code}`)
    }
  })
})

describe("mock known volumes", () => {
  it("T-ui-ipc-vcl-05 [Business rule] list merges volumes rows with ignored-only entries; forget refuses a disk with folders and removes the rest", async () => {
    // Arrange
    const withFolder: KnownVolume = { ...CAMERA, volumeKey: "vol-usb", folderCount: 1 }
    const api = createMockApi({
      count: 5,
      volumes: { known: [CAMERA, withFolder], ignored: [{ volumeKey: "vol-old", label: null, lastMountPoint: "H:\\", ignoredAt: 1 }] },
    })
    // Act + Assert: 병합 1행씩
    expect((await api.listKnownVolumes()).map((v) => [v.volumeKey, v.ignored, v.availability])).toEqual([
      ["vol-cam", false, "offline"],
      ["vol-usb", false, "offline"],
      ["vol-old", true, null],
    ])
    await expect(api.forgetVolumeRecord("vol-usb")).rejects.toMatchObject({ code: "volume_has_folders" })
    await api.forgetVolumeRecord("vol-cam")
    await api.forgetVolumeRecord("vol-old")
    expect((await api.listKnownVolumes()).map((v) => v.volumeKey)).toEqual(["vol-usb"])
    await expect(api.listIgnoredVolumes()).resolves.toEqual([])
    await expect(api.forgetVolumeRecord("vol-cam")).rejects.toMatchObject({ code: "not_found" })
  })
})
