// Categories: Happy (T-ui-ipc-detect-01) / Boundary (02 nulls) / External (03 reject passthrough) / Business rule (04 tauri api wiring)
import { invoke } from "@tauri-apps/api/core"
import { beforeEach, describe, expect, it, vi } from "vitest"
import { createTauriApi } from "@/lib/api/tauri"
import type { ChangeDetectorInfo, CommandError, RescanPolicy } from "@/lib/ipc-contract"
import { createMockApi } from "@/lib/api/mock"
import { listChangeDetectors, listFolders, setFolderRescanPolicy, setFolderWatchMode } from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

/** Rust 가 돌려주는 모양(camelCase) */
const INFO: ChangeDetectorInfo[] = [
  { id: "os_events", available: true, reasonKey: null, optionFields: [], experimental: false },
  { id: "scan_only", available: true, reasonKey: null, optionFields: [{ key: "intervalSec", kind: "integer", min: 10, max: 3600, default: 300 }], experimental: false },
]
const POLICY: RescanPolicy = { trigger: { kind: "schedule", days: ["mon", "fri"], at: "03:00" }, window: null, onStart: true }

beforeEach(() => {
  vi.mocked(invoke).mockReset()
})

describe("ipc change-detection commands", () => {
  it("T-ui-ipc-detect-01 [Happy] wrappers use the Rust command and argument names", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce(INFO).mockResolvedValueOnce({ folderId: 7 }).mockResolvedValueOnce({ folderId: 7 })
    // Act
    const list = await listChangeDetectors(7)
    await setFolderWatchMode(7, "scan_only", { intervalSec: 60 })
    await setFolderRescanPolicy(7, POLICY)
    // Assert
    expect(list).toEqual(INFO)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["list_change_detectors", { folderId: 7 }],
      ["set_folder_watch_mode", { folderId: 7, mode: "scan_only", options: { intervalSec: 60 } }],
      ["set_folder_rescan_policy", { folderId: 7, policy: POLICY }],
    ])
  })

  it("T-ui-ipc-detect-02 [Boundary] null mode / null options / null policy are sent as null (auto, defaults, global default)", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({ folderId: 7 })
    // Act
    await setFolderWatchMode(7, null, null)
    await setFolderRescanPolicy(7, null)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["set_folder_watch_mode", { folderId: 7, mode: null, options: null }],
      ["set_folder_rescan_policy", { folderId: 7, policy: null }],
    ])
  })

  it("T-ui-ipc-detect-03 [External] a CommandError reject passes through unchanged", async () => {
    // Arrange
    const err: CommandError = { code: "invalid_input", message: "invalid input: rescanPolicy.trigger.kind" }
    vi.mocked(invoke).mockRejectedValue(err)
    // Act / Assert
    await expect(setFolderRescanPolicy(7, POLICY)).rejects.toEqual(err)
  })

  it("T-ui-ipc-detect-04 [Business rule] the Tauri api routes the three methods to the wrappers", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue([])
    const api = createTauriApi()
    // Act
    await api.listChangeDetectors(3)
    await api.setFolderWatchMode(3, "os_events", null)
    await api.setFolderRescanPolicy(3, null)
    // Assert
    expect(vi.mocked(invoke).mock.calls.map((c) => c[0])).toEqual(["list_change_detectors", "set_folder_watch_mode", "set_folder_rescan_policy"])
  })

  it("T-ui-ipc-detect-05 [Happy] the Rust shapes for health, stall, experimental and ref/bool option kinds pass through unchanged", async () => {
    // Arrange: Rust 가 serde 로 내는 모양 그대로
    const info: ChangeDetectorInfo = {
      id: "ddp_audit",
      available: false,
      reasonKey: "remote_not_ready",
      optionFields: [{ key: "source", kind: "ref", min: 1, max: 9_007_199_254_740_991, default: 0 }],
      experimental: true,
    }
    const folder = {
      folderId: 1,
      watchMode: "ddp_audit",
      watchOptions: { source: 5, exclusiveOpenCheck: false },
      watchHealth: { state: "degraded", since: 7, reasonKey: "watch.health.remote_connect" },
      scanStall: { atPath: "\\\\nas\\a.mov", since: 9 },
    }
    vi.mocked(invoke).mockResolvedValueOnce([info]).mockResolvedValueOnce([folder])
    // Act
    const detectors = await listChangeDetectors(1)
    const folders = await listFolders()
    // Assert
    expect(detectors[0]).toEqual(info)
    expect(folders[0]).toMatchObject({ watchOptions: { source: 5, exclusiveOpenCheck: false }, watchHealth: { state: "degraded" }, scanStall: { since: 9 } })
  })

  it("T-ui-ipc-detect-06 [Validation] the mock follows the Rust option rules: auto takes only the common option, ref must name an existing connection of the right kind, defaults are not stored", async () => {
    // Arrange
    const api = createMockApi({ count: 5 })
    const ddp = await api.createRemoteSource({
      kind: "ddp_audit", name: "d", host: "h", port: 22, user: "u", authKind: "password", secret: "x", hostKey: null, logDir: null, dbPath: null, pollSec: 3,
      pathMap: [{ from: "/", to: "D:\\Photos" }],
    })
    const invalid = { code: "invalid_input", message: "invalid input: watchOptions" }
    // Act / Assert: 자동
    await expect(api.setFolderWatchMode(1, null, { exclusiveOpenCheck: 1 as unknown as boolean })).rejects.toMatchObject(invalid)
    await expect(api.setFolderWatchMode(1, null, { source: ddp.sourceId })).rejects.toMatchObject(invalid)
    expect((await api.setFolderWatchMode(1, null, { exclusiveOpenCheck: true })).watchOptions).toBeNull()
    // 참조: 없는 접속 · 종류가 다른 감지기
    await expect(api.setFolderWatchMode(1, "ddp_audit", { source: 999 })).rejects.toMatchObject(invalid)
    await expect(api.setFolderWatchMode(1, "ddp_audit", null)).resolves.toMatchObject({ watchOptions: null })
    await expect(api.setFolderWatchMode(1, "fanotify_feed", { source: ddp.sourceId })).rejects.toMatchObject(invalid)
    expect((await api.setFolderWatchMode(1, "ddp_audit", { source: ddp.sourceId, exclusiveOpenCheck: false })).watchOptions).toEqual({ source: ddp.sourceId, exclusiveOpenCheck: false })
    // 정수: 범위 밖 거부, 기본값(300)은 저장하지 않는다
    await expect(api.setFolderWatchMode(1, "polling", { intervalSec: 59 })).rejects.toMatchObject(invalid)
    expect((await api.setFolderWatchMode(1, "polling", { intervalSec: 300 })).watchOptions).toBeNull()
    expect((await api.setFolderWatchMode(1, "polling", { intervalSec: 120 })).watchOptions).toEqual({ intervalSec: 120 })
  })
})
