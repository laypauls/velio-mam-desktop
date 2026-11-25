// Categories: Happy (T-ui-ipc-eject-01) / External (02 error codes) / Business rule (03 mock same-disk flip, 04 idempotent + resume) / Validation (05 network, not_found)
import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import type { EjectPrepared } from "@/lib/ipc-contract"
import { prepareVolumeEject, resumeVolumeWatch } from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

const ALL = { image: true, video: true, audio: true }
/** Rust 가 돌려주는 모양(camelCase) */
const PREPARED: EjectPrepared = { mountPoint: "E:\\", folderIds: [4, 5] }

beforeEach(() => vi.mocked(invoke).mockReset())

describe("volume eject ipc", () => {
  it("T-ui-ipc-eject-01 [Happy] wrappers use the Rust command and argument names and return the server shape", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce(PREPARED).mockResolvedValueOnce(undefined)
    // Act
    const res = await prepareVolumeEject(4)
    await resumeVolumeWatch(4)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["prepare_volume_eject", { folderId: 4 }],
      ["resume_volume_watch", { folderId: 4 }],
    ])
    expect(res).toEqual(PREPARED)
  })

  it("T-ui-ipc-eject-02 [External] the Tauri api passes the commands through and keeps invalid_input / offline errors", async () => {
    // Arrange
    const api = createTauriApi()
    vi.mocked(invoke).mockResolvedValueOnce(PREPARED)
    // Act + Assert
    await expect(api.prepareVolumeEject(4)).resolves.toEqual(PREPARED)
    for (const code of ["invalid_input", "offline"]) {
      vi.mocked(invoke).mockRejectedValueOnce({ code, message: "x" })
      await expect(api.prepareVolumeEject(4)).rejects.toMatchObject({ code })
    }
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    await expect(api.resumeVolumeWatch(4)).resolves.toBeUndefined()
  })

  it("T-ui-ipc-eject-03 [Business rule] the mock flips ejectPrepared and availability for every folder on the same drive only", async () => {
    // Arrange
    const api = createMockApi({ count: 3 })
    await api.addFolder("E:\\data", ALL)
    await api.addFolder("E:\\more", ALL)
    // Act
    const res = await api.prepareVolumeEject(4)
    // Assert
    expect(res).toEqual({ mountPoint: "E:\\", folderIds: [4, 5] })
    const byPath = new Map((await api.listFolders()).map((f) => [f.path, f]))
    expect(byPath.get("E:\\data")).toMatchObject({ ejectPrepared: true, availability: "offline" })
    expect(byPath.get("E:\\more")).toMatchObject({ ejectPrepared: true, availability: "offline" })
    expect(byPath.get("D:\\Photos")).toMatchObject({ ejectPrepared: false, availability: "online" })
  })

  it("T-ui-ipc-eject-04 [Business rule] prepare is idempotent and resume restores the disk (and is a no-op when not prepared)", async () => {
    // Arrange
    const api = createMockApi({ count: 3 })
    await api.addFolder("E:\\data", ALL)
    // Act + Assert
    await api.prepareVolumeEject(4)
    await expect(api.prepareVolumeEject(4)).resolves.toMatchObject({ mountPoint: "E:\\" })
    await api.resumeVolumeWatch(4)
    expect((await api.listFolders()).find((f) => f.folderId === 4)).toMatchObject({ ejectPrepared: false, availability: "online" })
    await expect(api.resumeVolumeWatch(4)).resolves.toBeUndefined()
  })

  it("T-ui-ipc-eject-05 [Validation] the mock rejects network folders (invalid_input) and unknown ids (not_found)", async () => {
    // Arrange
    const api = createMockApi({ count: 3 })
    // Act + Assert: 3 = \\NAS\Archive (network, offline)
    await expect(api.prepareVolumeEject(3)).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.prepareVolumeEject(99)).rejects.toMatchObject({ code: "not_found" })
    await expect(api.resumeVolumeWatch(99)).rejects.toMatchObject({ code: "not_found" })
  })
})
