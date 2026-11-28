// Categories: Happy / External / Validation (command·event 이름 계약)
import { beforeEach, describe, expect, it, vi } from "vitest"
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import {
  backupNow,
  checkUpdate,
  getAppPaths,
  getAppInfo,
  getSettings,
  hideMainWindow,
  setTrayLabels,
  installUpdate,
  listBackups,
  onUpdateState,
  addFolder,
  jobSummary,
  listFolders,
  listScanErrors,
  listSubfolders,
  onJobsProgress,
  pauseJobs,
  pickDirectory,
  pickSaveFile,
  removeFolder,
  relocateFolder,
  rescanFolder,
  setFolderHidden,
  setFolderAlias,
  setFolderMediaTypes,
  countFolderAssetsByType,
  resumeJobs,
  retryScanErrors,
  JOBS_PROGRESS_EVENT,
  MAX_PAGE_LIMIT,
  EXPORT_PROGRESS_EVENT,
  addToCollection,
  collectionMembership,
  collectionPlayQueue,
  reorderCollection,
  copyToClipboard,
  createCollection,
  deleteCollection,
  deleteTag,
  exportAssets,
  listCollections,
  listTags,
  onExportProgress,
  openOriginal,
  removeFromCollection,
  renameCollection,
  renameTag,
  revealInExplorer,
  setFavorite,
  setMemo,
  setAssetTitle,
  updateAssetTags,
  type Collection,
  type ExportInput,
  type ExportProgress,
  type UpdateTagsInput,
  updateSettings,
  type AppPaths,
  type BackupInfo,
  type CommandError,
  type FolderNode,
  type JobSummary,
  type ManagedFolder,
  type Page,
  type ScanError,
  type Settings,
  type SettingsPatch,
  type AppInfo,
  type UpdateCheck,
  type UpdateError,
  type UpdateStatePayload,
} from "./ipc"
import type { CollectionMembership, PlayQueueItem, ReorderTarget } from "./ipc-contract"
import {
  changePassword,
  deleteOriginals,
  getProtectionStatus,
  moveOriginals,
  resetManagedData,
  setPassword,
  verifyPassword,
  type OriginalsResult,
  type ProtectionStatus,
  type ResetInput,
} from "./ipc"

import {
  clearRecentSearches,
  getAssetDetail,
  listRecentSearches,
  recordRecentSearch,
  searchAssets,
  searchIndex,
  type SearchIndex,
  type SearchFilters,
  type SearchPage,
  type SearchRequest,
  type SortSpec,
} from "./ipc"

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }))
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }))

describe("ipc.getAppInfo", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  it("T-ui-ipc-01 [Happy] invokes the exact 'app_info' command and returns its payload", async () => {
    // Arrange
    const info: AppInfo = { name: "velio.", version: "0.1.0" }
    vi.mocked(invoke).mockResolvedValue(info)
    // Act
    const result = await getAppInfo()
    // Assert
    expect(invoke).toHaveBeenCalledWith("app_info")
    expect(result).toEqual(info)
  })

  it("T-ui-ipc-02 [External] propagates invoke rejection", async () => {
    // Arrange
    vi.mocked(invoke).mockRejectedValue(new Error("core down"))
    // Act + Assert
    await expect(getAppInfo()).rejects.toThrow("core down")
  })
})

describe("ipc update commands", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
    vi.mocked(listen).mockReset()
  })

  it("T-ui-ipc-03 [Happy] checkUpdate invokes 'check_update' with no args and returns the camelCase payload", async () => {
    // Arrange
    const found: UpdateCheck = {
      available: true,
      currentVersion: "0.1.0",
      version: "0.1.1",
      notes: "fixes",
      date: null,
    }
    vi.mocked(invoke).mockResolvedValue(found)
    // Act
    const result = await checkUpdate()
    // Assert
    expect(invoke).toHaveBeenCalledWith("check_update")
    expect(result).toEqual(found)
  })

  it("T-ui-ipc-04 [External] checkUpdate propagates the structured Rust error", async () => {
    // Arrange
    const err: UpdateError = { code: "check_failed", message: "update check failed: offline" }
    vi.mocked(invoke).mockRejectedValue(err)
    // Act + Assert
    await expect(checkUpdate()).rejects.toEqual(err)
  })

  it("T-ui-ipc-05 [Happy] installUpdate invokes exactly 'install_update' only when called", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Assert (승인 전에는 호출되지 않는다)
    expect(invoke).not.toHaveBeenCalled()
    // Act
    await installUpdate()
    // Assert
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith("install_update")
  })

  it("T-ui-ipc-06 [External] installUpdate propagates 'previous_installer_missing' refusal", async () => {
    // Arrange
    const err: UpdateError = {
      code: "previous_installer_missing",
      message: "previous installer not found",
    }
    vi.mocked(invoke).mockRejectedValue(err)
    // Act + Assert
    await expect(installUpdate()).rejects.toEqual(err)
  })

  it("T-ui-ipc-07 [Validation] onUpdateState listens to 'update-state' and unwraps the payload", async () => {
    // Arrange
    const unlisten = vi.fn()
    let registered: ((e: { payload: UpdateStatePayload }) => void) | undefined
    vi.mocked(listen).mockImplementation(async (_name, cb) => {
      registered = cb as typeof registered
      return unlisten
    })
    const handler = vi.fn()
    // Act
    const off = await onUpdateState(handler)
    registered?.({ payload: { phase: "downloading", downloaded: 5, total: 10 } })
    // Assert
    expect(vi.mocked(listen).mock.calls[0][0]).toBe("update-state")
    expect(handler).toHaveBeenCalledWith({ phase: "downloading", downloaded: 5, total: 10 })
    expect(off).toBe(unlisten)
  })
})

describe("ipc storage commands (P2-1)", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  const settings: Settings = {
    dataDir: "C:\MAMData",
    rescanIntervalHours: 6,
    maxConcurrentScans: 3,
    authExemptMinutes: 5,
    backup: { enabled: true, intervalHours: 24, keep: 7, dir: null },
    closeToTray: true,
    firstRunCompleted: false,
    galleryCardDisplay: {
      info: { typeIcon: true, name: true, ext: true, resolution: true, size: true, date: true },
      dateBy: "captured",
      check: "hover",
      favorite: true,
      rating: true,
      duration: true,
      quickActions: true,
      cardSize: "m",
    },
    titleEditEnabled: false,
    originalActionsEnabled: false,
    sidebarRatingMenu: true,
    videoPlayerBuiltIn: true,
    hideShortcut: "Escape",
    closeViewerShortcut: "Q",
    slideshowImageSeconds: 5,
    sortFieldsEnabled: { capturedAt: true, modifiedAt: true, name: true, size: true, duration: true, rating: true },
    doubleClickAction: "preview",
    capturedAtFallback: "modified",
    similarMaxResults: 24,
    similarMinSimilarity: 0.75,
    galleryPaging: "infinite",
    galleryPageSize: 100,
    indexRail: "auto",
    theme: { mode: "system", systemDark: "dim" },
    language: "ko",
    mediaOpenAutoplay: false,
    playRangeKeys: "brackets",
    playRangesOnly: false,
    collectionPlayRangesOnly: false,
    subtitleVisible: true,
    generatedSubtitleVisible: true,
    subtitleDefaultLanguage: "ko",
    sceneCutsVisible: true,
    sceneDetectAuto: false,
    sceneDetectSensitivity: "medium",
    playerPlayingStepSec: 5,
    viewerPlacement: "right",
    layout: { detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104, sidebarSection: "library" },
    playerVolume: { level: 100, muted: false },
    resumeEnabled: true,
    resumeMinDurationSec: 180,
  }

  it("T-ui-ipc-08 [Happy] getSettings invokes 'get_settings' with no args", async () => {
    vi.mocked(invoke).mockResolvedValue(settings)
    expect(await getSettings()).toEqual(settings)
    expect(invoke).toHaveBeenCalledWith("get_settings")
  })

  it("T-ui-ipc-09 [Happy] updateSettings sends the partial patch under 'patch' (nested backup stays partial)", async () => {
    // Arrange
    const patch: SettingsPatch = { authExemptMinutes: 10, backup: { keep: 3 } }
    vi.mocked(invoke).mockResolvedValue({ ...settings, authExemptMinutes: 10 })
    // Act
    const result = await updateSettings(patch)
    // Assert
    expect(invoke).toHaveBeenCalledWith("update_settings", { patch })
    expect(result.authExemptMinutes).toBe(10)
  })

  it("T-ui-ipc-hide-01 [Happy] hideMainWindow invokes hide_main_window with no args; a rejection surfaces unchanged", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await hideMainWindow()
    // Assert
    expect(invoke).toHaveBeenCalledWith("hide_main_window")
    // External failure
    const err = { code: "internal", message: "boom" }
    vi.mocked(invoke).mockRejectedValue(err)
    await expect(hideMainWindow()).rejects.toEqual(err)
  })

  it("T-ui-ipc-tray-01 [Happy] setTrayLabels invokes set_tray_labels with { labels }; a rejection surfaces unchanged", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    const labels = { open: "Open", pauseJobs: "Pause jobs", resumeJobs: "Resume jobs", quit: "Quit", aiWaiting: "x" }
    // Act
    await setTrayLabels(labels)
    // Assert
    expect(invoke).toHaveBeenCalledWith("set_tray_labels", { labels })
    // External failure
    const err = { code: "internal", message: "boom" }
    vi.mocked(invoke).mockRejectedValue(err)
    await expect(setTrayLabels(labels)).rejects.toEqual(err)
  })

  it("T-ui-ipc-10 [Validation] a rejected patch surfaces the { code, message } error unchanged", async () => {
    // Arrange
    const err: CommandError = {
      code: "invalid_input",
      message: "invalid value for setting: authExemptMinutes",
    }
    vi.mocked(invoke).mockRejectedValue(err)
    // Act + Assert
    await expect(updateSettings({ authExemptMinutes: 99 })).rejects.toEqual(err)
  })

  it("T-ui-ipc-11 [Happy] backupNow / listBackups / getAppPaths use the exact command names", async () => {
    // Arrange
    const info: BackupInfo = {
      fileName: "mam-1.sqlite",
      createdAt: 1,
      sizeBytes: 4096,
      kind: "manual",
    }
    const paths: AppPaths = {
      dataDir: "C:\MAMData",
      databasePath: "C:\MAMData\database\mam.sqlite",
      backupsDir: "C:\MAMData\backups",
      logsDir: "C:\MAMData\logs",
    }
    vi.mocked(invoke).mockResolvedValueOnce(info)
    vi.mocked(invoke).mockResolvedValueOnce([info])
    vi.mocked(invoke).mockResolvedValueOnce(paths)
    // Act
    const created = await backupNow()
    const listed = await listBackups()
    const got = await getAppPaths()
    // Assert
    expect(vi.mocked(invoke).mock.calls.map((c) => c[0])).toEqual([
      "backup_now",
      "list_backups",
      "app_paths",
    ])
    expect([created, listed, got]).toEqual([info, [info], paths])
  })

  it("T-ui-ipc-12 [External] backupNow propagates disk_full", async () => {
    const err: CommandError = { code: "disk_full", message: "not enough disk space" }
    vi.mocked(invoke).mockRejectedValue(err)
    await expect(backupNow()).rejects.toEqual(err)
  })
})

const ALL_TYPES = { image: true, video: true, audio: true }

describe("ipc ingest commands (P2-2)", () => {
  const folder: ManagedFolder = {
    folderId: 1,
    path: "D:\Photos",
    volumeKind: "removable",
    volumeLabel: "USB",
    availability: "online",
    assetCount: 12,
    lastFullScanAt: 1_700_000_000_000,
    scanState: "idle",
    hidden: false,
    alias: null,
    mediaTypes: ALL_TYPES,
    ejectPrepared: false,
    ejectable: true,
    watchMode: null,
    watchOptions: null,
    watchActive: "os_events",
    watchFallbackKey: null,
    rescanPolicy: null,
    nextRescanAt: null,
  }
  const zero = { pending: 0, running: 0, failed: 0 }
  const summary: JobSummary = {
    paused: false,
    pausedKinds: [],
    counts: { scan: zero, meta: zero, thumb: zero, hash: zero, index: zero, ai: zero, scene: zero, plugin: zero },
    scan: null,
    totalAssets: 12,
  }

  beforeEach(() => {
    vi.mocked(invoke).mockReset()
    vi.mocked(listen).mockReset()
  })

  it("T-ui-ipc-13 [Happy] folder commands use the exact names and camelCase argument keys", async () => {
    // Arrange
    const node: FolderNode = { relPath: "2024", name: "2024", hasChildren: true, assetCount: 5 }
    vi.mocked(invoke).mockResolvedValueOnce([folder])
    vi.mocked(invoke).mockResolvedValueOnce(folder)
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockResolvedValueOnce([node])
    vi.mocked(invoke).mockResolvedValueOnce([node])
    // Act
    const listed = await listFolders()
    const added = await addFolder("D:\Photos", ALL_TYPES)
    await removeFolder(1)
    await rescanFolder(1)
    await setFolderHidden(1, true)
    const top = await listSubfolders(1, null)
    const sub = await listSubfolders(1, "2024")
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["list_folders"],
      ["add_folder", { path: "D:\Photos", mediaTypes: ALL_TYPES }],
      ["remove_folder", { folderId: 1 }],
      ["rescan_folder", { folderId: 1 }],
      ["set_folder_hidden", { folderId: 1, hidden: true }],
      ["list_subfolders", { folderId: 1, relPath: null }],
      ["list_subfolders", { folderId: 1, relPath: "2024" }],
    ])
    expect([listed, added, top, sub]).toEqual([[folder], folder, [node], [node]])
  })

  it("T-ui-hide-02 [External] setFolderHidden surfaces not_found unchanged", async () => {
    const err: CommandError = { code: "not_found", message: "folder" }
    vi.mocked(invoke).mockRejectedValueOnce(err)
    await expect(setFolderHidden(9, false)).rejects.toEqual(err)
  })

  it("T-ui-alias-01 [Happy] setFolderAlias invokes set_folder_alias with camelCase args and null clears", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(undefined).mockResolvedValueOnce(undefined)
    await setFolderAlias(1, "사진")
    await setFolderAlias(1, null)
    expect(vi.mocked(invoke).mock.calls.slice(-2)).toEqual([
      ["set_folder_alias", { folderId: 1, alias: "사진" }],
      ["set_folder_alias", { folderId: 1, alias: null }],
    ])
  })

  it("T-ui-alias-02 [External] setFolderAlias surfaces invalid_input unchanged", async () => {
    const err: CommandError = { code: "invalid_input", message: "alias" }
    vi.mocked(invoke).mockRejectedValueOnce(err)
    await expect(setFolderAlias(1, "x")).rejects.toEqual(err)
  })

  it("T-ui-mtype-01 [Happy] setFolderMediaTypes / countFolderAssetsByType invoke camelCase args and return payloads", async () => {
    const counts = { image: 3, video: 2, audio: 1 }
    vi.mocked(invoke).mockResolvedValueOnce({ detached: 3 }).mockResolvedValueOnce({ detached: 0 }).mockResolvedValueOnce(counts)
    const types = { image: false, video: true, audio: true }
    const purged = await setFolderMediaTypes(1, types, "purge")
    const kept = await setFolderMediaTypes(1, types, "keep")
    const got = await countFolderAssetsByType(1)
    expect(vi.mocked(invoke).mock.calls.slice(-3)).toEqual([
      ["set_folder_media_types", { folderId: 1, mediaTypes: types, mode: "purge" }],
      ["set_folder_media_types", { folderId: 1, mediaTypes: types, mode: "keep" }],
      ["count_folder_assets_by_type", { folderId: 1 }],
    ])
    expect([purged, kept, got]).toEqual([{ detached: 3 }, { detached: 0 }, counts])
  })

  it("T-ui-mtype-02 [Validation] media type commands surface invalid_input / not_found unchanged", async () => {
    const none = { image: false, video: false, audio: false }
    const bad: CommandError = { code: "invalid_input", message: "types" }
    const gone: CommandError = { code: "not_found", message: "folder" }
    vi.mocked(invoke).mockRejectedValueOnce(bad).mockRejectedValueOnce(bad).mockRejectedValueOnce(gone).mockRejectedValueOnce(gone)
    await expect(addFolder("D:" + String.fromCharCode(92) + "Photos", none)).rejects.toEqual(bad)
    await expect(setFolderMediaTypes(1, none, "keep")).rejects.toEqual(bad)
    await expect(setFolderMediaTypes(9, ALL_TYPES, "keep")).rejects.toEqual(gone)
    await expect(countFolderAssetsByType(9)).rejects.toEqual(gone)
  })

  it("T-ui-ipc-14 [Validation] addFolder surfaces already_registered / nested_folder / offline unchanged", async () => {
    for (const code of ["already_registered", "nested_folder", "offline"] as const) {
      const err: CommandError = { code, message: code }
      vi.mocked(invoke).mockRejectedValueOnce(err)
      await expect(addFolder("D:\Photos", ALL_TYPES)).rejects.toEqual(err)
    }
  })

  it("T-ui-ipc-25 [Happy] relocateFolder invokes relocate_folder with { folderId, newPath } and returns the result", async () => {
    // Arrange
    const result = { folder: { ...folder, path: "E:\Photos" }, relocated: 12 }
    vi.mocked(invoke).mockResolvedValueOnce(result)
    // Act
    const out = await relocateFolder(1, "E:\Photos")
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([["relocate_folder", { folderId: 1, newPath: "E:\Photos" }]])
    expect(out).toEqual(result)
  })

  it("T-ui-ipc-26 [Validation] relocateFolder surfaces scan_running / already_registered / nested_folder / offline unchanged", async () => {
    for (const code of ["scan_running", "already_registered", "nested_folder", "offline"] as const) {
      const err: CommandError = { code, message: code }
      vi.mocked(invoke).mockRejectedValueOnce(err)
      await expect(relocateFolder(1, "E:\Photos")).rejects.toEqual(err)
    }
  })

  it("T-ui-ipc-15 [Happy] job commands send { kind } (null = all) and return the summary", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue({ ...summary, paused: true })
    // Act
    const a = await pauseJobs(null)
    await pauseJobs("thumb")
    await resumeJobs(null)
    const s = await jobSummary()
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["pause_jobs", { kind: null }],
      ["pause_jobs", { kind: "thumb" }],
      ["resume_jobs", { kind: null }],
      ["job_summary"],
    ])
    expect(a.paused).toBe(true)
    expect(s.counts.scan).toEqual(zero)
  })

  it("T-ui-ipc-16 [Happy] scan error list/retry pass cursor, limit and errorIds", async () => {
    // Arrange
    const item: ScanError = {
      errorId: 3,
      folderId: 1,
      path: "D:\Photos\locked",
      errorCode: "permission_denied",
      message: "denied",
      attempts: 2,
      lastAt: 1,
    }
    const page: Page<ScanError> = { items: [item], total: 1, nextCursor: null }
    vi.mocked(invoke).mockResolvedValueOnce(page)
    vi.mocked(invoke).mockResolvedValueOnce(1)
    vi.mocked(invoke).mockResolvedValueOnce(0)
    // Act
    const got = await listScanErrors(null, MAX_PAGE_LIMIT)
    const retried = await retryScanErrors([3])
    const all = await retryScanErrors(null)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["list_scan_errors", { cursor: null, limit: 200 }],
      ["retry_scan_errors", { errorIds: [3] }],
      ["retry_scan_errors", { errorIds: null }],
    ])
    expect([got, retried, all]).toEqual([page, 1, 0])
  })

  it("T-ui-ipc-17 [Happy] onJobsProgress listens to 'jobs://progress', unwraps the payload and returns the unlisten", async () => {
    // Arrange
    const unlisten = vi.fn()
    vi.mocked(listen).mockResolvedValue(unlisten)
    const seen: JobSummary[] = []
    // Act
    const off = await onJobsProgress((s) => seen.push(s))
    const [name, cb] = vi.mocked(listen).mock.calls[0]
    ;(cb as (e: { payload: JobSummary }) => void)({ payload: summary })
    // Assert
    expect(name).toBe(JOBS_PROGRESS_EVENT)
    expect(JOBS_PROGRESS_EVENT).toBe("jobs://progress")
    expect(seen).toEqual([summary])
    expect(off).toBe(unlisten)
  })

  it("T-ui-ipc-18 [Happy] pickDirectory invokes 'pick_directory' and returns the path or null on cancel", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce("D:\Export")
    vi.mocked(invoke).mockResolvedValueOnce(null)
    // Act
    const picked = await pickDirectory()
    const cancelled = await pickDirectory()
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([["pick_directory"], ["pick_directory"]])
    expect([picked, cancelled]).toEqual(["D:\Export", null])
  })

  it("T-ui-ipc-19 [External] pickDirectory propagates a core error", async () => {
    const err: CommandError = { code: "internal", message: "internal error" }
    vi.mocked(invoke).mockRejectedValue(err)
    await expect(pickDirectory()).rejects.toEqual(err)
  })

  it("T-ui-ipc-pick-save-01 [Happy] pickSaveFile invokes 'pick_save_file', omitting undefined args", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce("D:\\out\\a.csv").mockResolvedValueOnce(null).mockResolvedValueOnce(null).mockResolvedValueOnce(null)
    const filters = [{ name: "CSV", extensions: ["csv"] }]
    // Act
    const picked = await pickSaveFile("a.csv", filters)
    const cancelled = await pickSaveFile()
    await pickSaveFile("a.csv")
    await pickSaveFile(undefined, filters)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["pick_save_file", { defaultFileName: "a.csv", filters }],
      ["pick_save_file"],
      ["pick_save_file", { defaultFileName: "a.csv" }],
      ["pick_save_file", { filters }],
    ])
    expect([picked, cancelled]).toEqual(["D:\\out\\a.csv", null])
  })

  it("T-ui-ipc-pick-save-02 [External] pickSaveFile propagates a core error", async () => {
    const err: CommandError = { code: "invalid_input", message: "bad file name" }
    vi.mocked(invoke).mockRejectedValue(err)
    await expect(pickSaveFile("a")).rejects.toEqual(err)
  })
})

describe("ipc P2-7 user data / P2-8 reuse", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
    vi.mocked(listen).mockReset()
  })

  it("T-ui-ipc-title-01 [Happy] setAssetTitle sends assetId + title (null clears)", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)
    await setAssetTitle(5, "제주 일출")
    await setAssetTitle(5, null)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["set_asset_title", { assetId: 5, title: "제주 일출" }],
      ["set_asset_title", { assetId: 5, title: null }],
    ])
  })

  it("T-ui-ipc-title-02 [External] setAssetTitle propagates the backend rejection", async () => {
    const err = { code: "invalid_input", message: "title editing disabled" }
    vi.mocked(invoke).mockRejectedValue(err)
    await expect(setAssetTitle(5, "x")).rejects.toEqual(err)
  })

  it("T-ui-ipc-20 [Happy] user data commands send the contract argument names", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    const input: UpdateTagsInput = { assetIds: [1, 2], add: ["여행"], remove: [3] }
    // Act
    await listTags()
    await updateAssetTags(input)
    await renameTag(3, "새 이름")
    await deleteTag(3)
    await setMemo(5, "메모")
    await setFavorite([1, 2], true)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["list_tags"],
      ["update_asset_tags", { input }],
      ["rename_tag", { tagId: 3, name: "새 이름" }],
      ["delete_tag", { tagId: 3 }],
      ["set_memo", { assetId: 5, memo: "메모" }],
      ["set_favorite", { assetIds: [1, 2], favorite: true }],
    ])
  })

  it("T-ui-ipc-21 [Happy] collection commands send the contract argument names and return the core value", async () => {
    // Arrange
    const col: Collection = { collectionId: 4, name: "묶음", assetCount: 0, createdAt: 1 }
    vi.mocked(invoke).mockResolvedValue(col)
    // Act
    const created = await createCollection("묶음")
    const renamed = await renameCollection(4, "새")
    await listCollections()
    await deleteCollection(4)
    await addToCollection(4, [1])
    await removeFromCollection(4, [1])
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["create_collection", { name: "묶음" }],
      ["rename_collection", { collectionId: 4, name: "새" }],
      ["list_collections"],
      ["delete_collection", { collectionId: 4 }],
      ["add_to_collection", { collectionId: 4, assetIds: [1] }],
      ["remove_from_collection", { collectionId: 4, assetIds: [1] }],
    ])
    expect([created, renamed]).toEqual([col, col])
  })

  it("T-ui-ipc-22 [Validation] a duplicate name rejects with the core error code unchanged", async () => {
    const err: CommandError = { code: "already_registered", message: "already registered" }
    vi.mocked(invoke).mockRejectedValue(err)
    await expect(createCollection("중복")).rejects.toEqual(err)
  })

  it("T-ui-ipc-23 [Happy] exportAssets wraps the input and onExportProgress unwraps the export progress event", async () => {
    // Arrange
    const input: ExportInput = { assetIds: [1], destDir: "D:\\out", conflict: "rename", includeManifest: true }
    vi.mocked(invoke).mockResolvedValue({ taskId: "export-1" })
    const unlisten = vi.fn()
    vi.mocked(listen).mockResolvedValue(unlisten)
    const seen: ExportProgress[] = []
    const progress: ExportProgress = { taskId: "export-1", done: 1, total: 1, failed: 0, skipped: 0, finished: true }
    // Act
    const started = await exportAssets(input)
    const off = await onExportProgress((p) => seen.push(p))
    const [name, cb] = vi.mocked(listen).mock.calls[0]
    ;(cb as (e: { payload: ExportProgress }) => void)({ payload: progress })
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([["export_assets", { input }]])
    expect(started).toEqual({ taskId: "export-1" })
    expect(name).toBe(EXPORT_PROGRESS_EVENT)
    expect(EXPORT_PROGRESS_EVENT).toBe("export://progress")
    expect(seen).toEqual([progress])
    expect(off).toBe(unlisten)
  })

  it("T-ui-ipc-24 [Happy] clipboard/open/reveal send contract names; offline errors propagate", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockRejectedValueOnce({ code: "offline", message: "volume is offline" } satisfies CommandError)
    // Act
    await copyToClipboard([1, 2])
    await openOriginal(7)
    const failed = revealInExplorer(7)
    // Assert
    await expect(failed).rejects.toEqual({ code: "offline", message: "volume is offline" })
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["copy_to_clipboard", { assetIds: [1, 2] }],
      ["open_original", { assetId: 7 }],
      ["reveal_in_explorer", { assetId: 7 }],
    ])
  })
})

describe("ipc P2-9 protection & operations", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  it("T-ui-ipc-30 [Happy] password commands use the contract names and argument keys", async () => {
    // Arrange
    const status: ProtectionStatus = { passwordSet: true, authValidUntil: 1_800_000_300_000 }
    vi.mocked(invoke).mockResolvedValue(status)
    // Act
    const results = [
      await getProtectionStatus(),
      await setPassword("abcdefgh1"),
      await changePassword("abcdefgh1", "zyxwvuts2"),
      await verifyPassword("zyxwvuts2"),
    ]
    // Assert
    expect(results).toEqual([status, status, status, status])
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["get_protection_status"],
      ["set_password", { password: "abcdefgh1" }],
      ["change_password", { current: "abcdefgh1", next: "zyxwvuts2" }],
      ["verify_password", { password: "zyxwvuts2" }],
    ])
  })

  it("T-ui-ipc-31 [Validation] auth and lockout errors reach the caller unchanged (incl. too_many_attempts)", async () => {
    // Arrange
    const codes: CommandError["code"][] = [
      "password_incorrect",
      "too_many_attempts",
      "password_not_set",
      "password_already_set",
      "auth_required",
      "invalid_input",
    ]
    for (const code of codes) vi.mocked(invoke).mockRejectedValueOnce({ code, message: "m" } satisfies CommandError)
    // Act + Assert
    await expect(verifyPassword("x")).rejects.toEqual({ code: "password_incorrect", message: "m" })
    await expect(verifyPassword("x")).rejects.toEqual({ code: "too_many_attempts", message: "m" })
    await expect(changePassword("a", "b")).rejects.toMatchObject({ code: "password_not_set" })
    await expect(setPassword("abcdefgh1")).rejects.toMatchObject({ code: "password_already_set" })
    await expect(deleteOriginals([1])).rejects.toMatchObject({ code: "auth_required" })
    await expect(moveOriginals([1], "D:/x")).rejects.toMatchObject({ code: "invalid_input" })
  })

  it("T-ui-ipc-32 [Happy] originals and reset commands send contract arguments and return the per-asset result", async () => {
    // Arrange
    const result: OriginalsResult = { succeeded: 1, failed: [{ assetId: 2, message: "locked" }] }
    vi.mocked(invoke).mockResolvedValueOnce(result)
    vi.mocked(invoke).mockResolvedValueOnce(result)
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    vi.mocked(invoke).mockResolvedValueOnce(undefined)
    const known: ResetInput = { password: "abcdefgh1", forgotPassword: false }
    const forgot: ResetInput = { password: null, forgotPassword: true }
    // Act
    const deleted = await deleteOriginals([1, 2])
    const moved = await moveOriginals([1, 2], "D:/Archive")
    await resetManagedData(known)
    await resetManagedData(forgot)
    // Assert
    expect(deleted).toEqual(result)
    expect(moved).toEqual(result)
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["delete_originals", { assetIds: [1, 2] }],
      ["move_originals", { assetIds: [1, 2], destDir: "D:/Archive" }],
      ["reset_managed_data", { input: known }],
      ["reset_managed_data", { input: forgot }],
    ])
  })
})

describe("ipc P2-5 search & detail", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  const filters: SearchFilters = {
    mediaTypes: ["video"],
    includeOffline: true,
    includeMissing: false,
    capturedFrom: null,
    capturedTo: null,
    sizeMin: null,
    sizeMax: null,
    durationMinMs: null,
    durationMaxMs: null,
    folder: { folderId: 1, relPath: "2024", recursive: true },
    favoritesOnly: false,
    ratingMin: null,
    ratingMax: null,
    collectionId: null,
    tagIds: [3],
  }
  const sort: SortSpec = { field: "capturedAt", direction: "desc" }

  it("T-ui-ipc-40 [Happy] search_assets sends { req } and returns the page (incl. optional mode / degraded)", async () => {
    // Arrange
    const req: SearchRequest = { query: "beach", filters, sort, limit: 50, cursor: null, mode: "hybrid" }
    const page: SearchPage = { items: [], total: 0, nextCursor: null, prevCursor: null, startOffset: 0, generation: 7, totalIsEstimate: false, tookMs: 3, degraded: true }
    vi.mocked(invoke).mockResolvedValue(page)
    // Act
    const result = await searchAssets(req)
    // Assert
    expect(invoke).toHaveBeenCalledWith("search_assets", { req })
    expect(result).toEqual(page)
  })

  it("T-ui-ipc-40b [Happy] search_assets carries direction / at, and search_index sends { req } with the sections shape", async () => {
    // Arrange
    const base: SearchRequest = { query: "", filters, sort, limit: 100, cursor: null }
    const backward: SearchRequest = { ...base, cursor: "c1.x", direction: "backward" }
    const seek: SearchRequest = { ...base, at: 5000 }
    const index: SearchIndex = {
      total: 3,
      generation: 7,
      kind: "month",
      sections: [
        { key: "2024-05", label: null, count: 2 },
        { key: "none", label: null, count: 1 },
      ],
    }
    vi.mocked(invoke).mockResolvedValue(index)
    // Act
    await searchAssets(backward)
    await searchAssets(seek)
    const result = await searchIndex({ query: "", filters, sort })
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["search_assets", { req: backward }],
      ["search_assets", { req: seek }],
      ["search_index", { req: { query: "", filters, sort } }],
    ])
    expect(result).toEqual(index)
  })

  it("T-ui-ipc-40c [External] search_index rejection propagates the command error unchanged", async () => {
    // Arrange
    vi.mocked(invoke).mockRejectedValue({ code: "index_not_ready", message: "search index is not ready" })
    // Act + Assert
    await expect(searchIndex({ query: "", filters, sort })).rejects.toEqual({ code: "index_not_ready", message: "search index is not ready" })
  })

  it("T-ui-ipc-41 [Happy] detail and history commands use the contract command and argument names", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    // Act
    await getAssetDetail(7)
    await listRecentSearches()
    await recordRecentSearch("beach", filters)
    await clearRecentSearches()
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["get_asset_detail", { assetId: 7 }],
      ["list_recent_searches"],
      ["record_recent_search", { query: "beach", filters }],
      ["clear_recent_searches"],
    ])
  })

  it("T-ui-ipc-42 [Validation] stale cursor and not-ready errors reach the caller unchanged", async () => {
    // Arrange
    const stale: CommandError = { code: "invalid_input", message: "invalid input: stale cursor" }
    const notReady: CommandError = { code: "index_not_ready", message: "search index is not ready" }
    vi.mocked(invoke).mockRejectedValueOnce(stale)
    vi.mocked(invoke).mockRejectedValueOnce(notReady)
    const req: SearchRequest = { query: "", filters, sort, limit: 50, cursor: "c1.x" }
    // Act + Assert
    await expect(searchAssets(req)).rejects.toEqual(stale)
    await expect(searchAssets({ ...req, cursor: null })).rejects.toEqual(notReady)
  })

  it("T-ui-ipc-46 [Happy] reorder_collection sends collectionId, assetIds and each of the three target forms", async () => {
    // Arrange
    vi.mocked(invoke).mockResolvedValue(undefined)
    const targets: ReorderTarget[] = [{ kind: "before", assetId: 9 }, { kind: "start" }, { kind: "end" }]
    // Act
    for (const target of targets) await reorderCollection(4, [1, 2], target)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual(targets.map((target) => ["reorder_collection", { collectionId: 4, assetIds: [1, 2], target }]))
  })

  it("T-ui-ipc-47 [Happy] collection_membership and collection_play_queue use the contract names and return the camelCase payloads", async () => {
    // Arrange
    const membership: CollectionMembership[] = [{ collectionId: 4, count: 2 }]
    const queue: PlayQueueItem[] = [{ assetId: 1, mediaType: "video", availability: "online", fileName: "a.mp4", title: null, durationMs: 1200 }]
    vi.mocked(invoke).mockResolvedValueOnce(membership).mockResolvedValueOnce(queue)
    // Act
    const m = await collectionMembership([1, 2])
    const q = await collectionPlayQueue(4)
    // Assert
    expect(vi.mocked(invoke).mock.calls).toEqual([
      ["collection_membership", { assetIds: [1, 2] }],
      ["collection_play_queue", { collectionId: 4 }],
    ])
    expect([m, q]).toEqual([membership, queue])
  })

  it("T-ui-ipc-48 [Validation] collectionOrder travels in the search request unchanged, and a missing-collection rejection reaches the caller", async () => {
    // Arrange
    const req: SearchRequest = { query: "", filters, sort: { field: "collectionOrder", direction: "asc" }, limit: 50, cursor: null }
    const err: CommandError = { code: "invalid_input", message: "sort" }
    vi.mocked(invoke).mockRejectedValueOnce(err)
    // Act + Assert
    await expect(searchAssets(req)).rejects.toEqual(err)
    expect(invoke).toHaveBeenCalledWith("search_assets", { req })
    // 재배치 실패(없는 컬렉션 등)도 코드 그대로
    const notFound: CommandError = { code: "not_found", message: "not found" }
    vi.mocked(invoke).mockRejectedValueOnce(notFound)
    await expect(reorderCollection(99, [1], { kind: "end" })).rejects.toEqual(notFound)
  })

  it("T-ui-ipc-49 [Happy] updateSettings carries slideshowImageSeconds and the settings type includes it", async () => {
    // Arrange
    const patch: SettingsPatch = { slideshowImageSeconds: 12 }
    vi.mocked(invoke).mockResolvedValue({ slideshowImageSeconds: 12 })
    // Act
    const result = await updateSettings(patch)
    // Assert
    expect(invoke).toHaveBeenCalledWith("update_settings", { patch: { slideshowImageSeconds: 12 } })
    expect(result.slideshowImageSeconds).toBe(12)
  })
})
