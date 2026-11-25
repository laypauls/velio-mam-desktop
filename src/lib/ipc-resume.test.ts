// Categories: Happy (command 이름·인자 모양) / Boundary (startMs 생략·null) / Business rule (mock 저장 판정) / External (reject 전파)
import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createMockApi } from "@/lib/api/mock"
import { createTauriApi } from "@/lib/api/tauri"
import { playerOpen, resumeClearAll, resumeClearAssets, resumeClearFolder, resumeCountAssets, resumeGet, resumeSave, resumeSetFolderEnabled, resumeStats } from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

const CANVAS = { width: 800, height: 450 }
const OPEN_REPLY = { sessionId: 4, lengthMs: 83_000, fps: 25, state: "paused", timeMs: 0 } as const

beforeEach(() => {
  vi.mocked(invoke).mockReset()
})

describe("ipc resume commands", () => {
  it("T-ui-ipc-resume-01 [Happy] resume_get / resume_save send camelCase args and return Rust-shaped values", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce({ positionMs: 61_000, durationMs: 600_000 }).mockResolvedValueOnce("saved").mockResolvedValueOnce(null)
    // Act
    const got = await resumeGet(7)
    const outcome = await resumeSave(7, 62_000, 600_000)
    const none = await resumeGet(8)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["resume_get", { assetId: 7 }],
      ["resume_save", { assetId: 7, positionMs: 62_000, durationMs: 600_000 }],
      ["resume_get", { assetId: 8 }],
    ])
    expect(got).toEqual({ positionMs: 61_000, durationMs: 600_000 })
    expect(outcome).toBe("saved")
    expect(none).toBeNull()
  })

  it("T-ui-ipc-resume-02 [Boundary] resume_save sends durationMs: null explicitly when the length is unknown", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue("skipped")
    // Act
    await resumeSave(3, 10_000, null)
    // Assert
    expect(invoke).toHaveBeenCalledWith("resume_save", { assetId: 3, positionMs: 10_000, durationMs: null })
  })

  it("T-ui-ipc-resume-03 [Happy] clear / count / stats commands use the contract names and arg objects", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce(2).mockResolvedValueOnce(2).mockResolvedValueOnce(5).mockResolvedValueOnce(9).mockResolvedValueOnce({ folders: [{ folderId: 1, count: 4, enabled: true }], total: 9 })
    // Act
    const counted = await resumeCountAssets([1, 2, 3])
    const cleared = await resumeClearAssets([1, 2])
    const folder = await resumeClearFolder(1)
    const all = await resumeClearAll()
    const stats = await resumeStats()
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["resume_count_assets", { assetIds: [1, 2, 3] }],
      ["resume_clear_assets", { assetIds: [1, 2] }],
      ["resume_clear_folder", { folderId: 1 }],
      ["resume_clear_all"],
      ["resume_stats"],
    ])
    expect([counted, cleared, folder, all]).toEqual([2, 2, 5, 9])
    expect(stats).toEqual({ folders: [{ folderId: 1, count: 4, enabled: true }], total: 9 })
  })

  it("T-ui-ipc-resume-12 [Happy] resume_set_folder_enabled sends camelCase args through the wrapper and the tauri adapter; errors propagate", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await resumeSetFolderEnabled(3, false)
    await createTauriApi().resumeSetFolderEnabled(4, true)
    vi.mocked(invoke).mockRejectedValueOnce({ code: "not_found", message: "folder not found" })
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["resume_set_folder_enabled", { folderId: 3, enabled: false }],
      ["resume_set_folder_enabled", { folderId: 4, enabled: true }],
    ])
    await expect(resumeSetFolderEnabled(999, true)).rejects.toEqual({ code: "not_found", message: "folder not found" })
  })

  it("T-ui-ipc-resume-04 [External] command errors propagate as the structured CommandError", async () => {
    // Arrange
    vi.mocked(invoke).mockRejectedValue({ code: "not_found", message: "asset not found" })
    // Act + Assert
    await expect(resumeSave(999, 1, null)).rejects.toEqual({ code: "not_found", message: "asset not found" })
    await expect(resumeClearFolder(999)).rejects.toEqual({ code: "not_found", message: "asset not found" })
  })
})

describe("ipc player_open startMs", () => {
  it("T-ui-ipc-resume-05 [Happy] startMs is sent only when non-null; existing calls keep the exact same payload", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(OPEN_REPLY)
    const ranges = [{ startMs: 1000, endMs: 2000 }]
    // Act
    await playerOpen(7, CANVAS, false)
    await playerOpen(7, CANVAS, false, null)
    await playerOpen(7, CANVAS, true, undefined, 61_000)
    await playerOpen(7, CANVAS, true, ranges, 61_000)
    await playerOpen(7, CANVAS, true, ranges, null)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["player_open", { assetId: 7, autoplay: false, canvas: CANVAS }],
      ["player_open", { assetId: 7, autoplay: false, canvas: CANVAS, ranges: null }],
      ["player_open", { assetId: 7, autoplay: true, canvas: CANVAS, startMs: 61_000 }],
      ["player_open", { assetId: 7, autoplay: true, canvas: CANVAS, ranges, startMs: 61_000 }],
      ["player_open", { assetId: 7, autoplay: true, canvas: CANVAS, ranges }],
    ])
  })

  it("T-ui-ipc-resume-06 [Happy] the tauri api adapter forwards startMs and the resume commands", async () => {
    // Arrange
    const api = createTauriApi()
    vi.mocked(invoke).mockResolvedValue(OPEN_REPLY)
    // Act
    await api.playerOpen(1, CANVAS, true, undefined, 5000)
    vi.mocked(invoke).mockResolvedValueOnce(3)
    const n = await api.resumeCountAssets([1])
    // Assert
    expect(vi.mocked(invoke).mock.calls[0]).toEqual(["player_open", { assetId: 1, autoplay: true, canvas: CANVAS, startMs: 5000 }])
    expect(vi.mocked(invoke).mock.calls[1]).toEqual(["resume_count_assets", { assetIds: [1] }])
    expect(n).toBe(3)
  })
})

describe("mock api resume (mirrors resume_save rules)", () => {
  const LONG = 600_000

  it("T-ui-ipc-resume-07 [Business rule] mock settings default resumeEnabled true / resumeMinDurationSec 180", async () => {
    // Arrange + Act
    const s = await createMockApi({ count: 5 }).getSettings()
    // Assert
    expect(s.resumeEnabled).toBe(true)
    expect(s.resumeMinDurationSec).toBe(180)
  })

  it("T-ui-ipc-resume-08 [Business rule] save → saved, get returns it; <5s or within the end window clears; short content skips", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act + Assert
    expect(await api.resumeSave(1, 60_000, LONG)).toBe("saved")
    expect(await api.resumeGet(1)).toEqual({ positionMs: 60_000, durationMs: LONG })
    expect(await api.resumeSave(1, 4_999, LONG)).toBe("cleared") // 처음 5초
    expect(await api.resumeGet(1)).toBeNull()
    expect(await api.resumeSave(1, 60_000, LONG)).toBe("saved")
    expect(await api.resumeSave(1, LONG - 15_000, LONG)).toBe("cleared") // 끝 15초 (5% = 30초 이내)
    expect(await api.resumeSave(2, 60_000, 120_000)).toBe("skipped") // 최소 길이 3분 미만
    expect(await api.resumeSave(2, 60_000, null)).toBe("skipped") // 길이 미상
  })

  it("T-ui-ipc-resume-09 [Business rule] disabled → skipped and the existing record is kept", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    await api.resumeSave(1, 60_000, LONG)
    await api.updateSettings({ resumeEnabled: false })
    // Act
    const outcome = await api.resumeSave(1, 90_000, LONG)
    // Assert
    expect(outcome).toBe("skipped")
    expect((await api.resumeGet(1))?.positionMs).toBe(60_000)
  })

  it("T-ui-ipc-resume-10 [Business rule] count / clear selected / clear folder / clear all / stats agree", async () => {
    // Arrange
    const api = createMockApi({ count: 20 })
    for (const id of [1, 2, 3]) await api.resumeSave(id, 60_000, LONG)
    // Act + Assert
    expect(await api.resumeCountAssets([1, 2, 9])).toBe(2)
    const stats = await api.resumeStats()
    expect(stats.total).toBe(3)
    expect(stats.folders.reduce((n, f) => n + f.count, 0)).toBe(3)
    expect(stats.folders.length).toBeGreaterThan(0)
    expect(await api.resumeClearAssets([1, 9])).toBe(1)
    const folderOf2 = stats.folders.find((f) => f.count > 0)!.folderId
    expect(await api.resumeClearFolder(folderOf2)).toBeGreaterThanOrEqual(1)
    expect(await api.resumeClearAll()).toBeGreaterThanOrEqual(0)
    expect((await api.resumeStats()).total).toBe(0)
  })

  it("T-ui-ipc-resume-11 [Validation] negative position, unknown asset and unknown folder are rejected", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    // Act + Assert
    await expect(api.resumeSave(1, -1, LONG)).rejects.toMatchObject({ code: "invalid_input" })
    await expect(api.resumeSave(99_999, 10_000, LONG)).rejects.toMatchObject({ code: "not_found" })
    await expect(api.resumeClearFolder(99_999)).rejects.toMatchObject({ code: "not_found" })
    await expect(api.resumeSetFolderEnabled(99_999, false)).rejects.toMatchObject({ code: "not_found" })
  })

  it("T-ui-ipc-resume-13 [Business rule] a disabled folder: save → skipped (record kept), get → null, stats enabled false; other folders unaffected; re-enable restores", async () => {
    // Arrange
    const api = createMockApi({ count: 20 })
    await api.resumeSave(1, 60_000, LONG)
    const folders = (await api.resumeStats()).folders
    expect(folders.every((f) => f.enabled)).toBe(true)
    const folderOf1 = folders.find((f) => f.count > 0)!.folderId
    // Act
    await api.resumeSetFolderEnabled(folderOf1, false)
    // Assert
    expect(await api.resumeGet(1)).toBeNull()
    expect(await api.resumeSave(1, 90_000, LONG)).toBe("skipped")
    const after = await api.resumeStats()
    expect(after.folders.find((f) => f.folderId === folderOf1)).toMatchObject({ count: 1, enabled: false })
    expect(after.folders.filter((f) => f.folderId !== folderOf1).every((f) => f.enabled)).toBe(true)
    await api.resumeSetFolderEnabled(folderOf1, true)
    expect((await api.resumeGet(1))?.positionMs).toBe(60_000)
  })
})
