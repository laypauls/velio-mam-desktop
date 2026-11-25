import type {
  FolderNode,
  JobKind,
  JobSummary,
  ManagedFolder,
  MediaTypeSet,
  ScanError,
} from "@/lib/ipc-contract"
import type { MockCtx } from "./mock-ctx"
import type { MockDb } from "./mock-data"
import { cmdError } from "./mock-search"
import { mockWatchFields } from "./mock-watch"
import type { UiApi } from "./types"

const LAST_FULL_SCAN_AT = Date.UTC(2026, 9, 20)
const ALL_TYPES: MediaTypeSet = { image: true, video: true, audio: true }
const noneSelected = (m: MediaTypeSet) => !m.image && !m.video && !m.audio

const JOB_KINDS: JobKind[] = ["scan", "meta", "thumb", "hash", "index", "ai", "scene", "plugin"]

type OpsApi = Pick<
  UiApi,
  | "listFolders" | "addFolder" | "relocateFolder" | "removeFolder" | "rescanFolder" | "setFolderHidden" | "setFolderAlias" | "setFolderMediaTypes" | "countFolderAssetsByType" | "listSubfolders" | "jobSummary"
  | "pauseJobs" | "resumeJobs" | "listScanErrors" | "retryScanErrors" | "onJobsProgress"
>

/** P2-2 관리 폴더·작업 요약 mock. */
/** 관리 폴더 mock 레코드 → `ManagedFolder` (변경 감지 필드는 `mock-watch` 가 채운다) */
export function folderViewOf({ db, settings, now }: Pick<MockCtx, "db" | "settings" | "now">): (f: MockDb["folders"][number]) => ManagedFolder {
  return (f) => ({
    folderId: f.folderId,
    path: f.path,
    volumeKind: f.volumeKind,
    volumeLabel: f.volumeLabel,
    availability: f.offline || f.ejectPrepared === true ? "offline" : "online",
    assetCount: db().recs.reduce((n, r) => n + (r.folderId === f.folderId && r.status === "active" ? 1 : 0), 0),
    lastFullScanAt: LAST_FULL_SCAN_AT,
    scanState: "idle",
    hidden: f.hidden === true,
    alias: f.alias ?? null,
    mediaTypes: f.mediaTypes ?? ALL_TYPES,
    ejectPrepared: f.ejectPrepared === true,
    ejectable: f.ejectable !== false,
    ...mockWatchFields(f, { settingsHours: settings.rescanIntervalHours, lastFullScanAt: LAST_FULL_SCAN_AT, now: now() }),
  })
}

export function createMockOps({ db, wait, opts, count, settings, now }: MockCtx): OpsApi {
  const jobPaused = { global: false, kinds: new Set<JobKind>() }
  const jobHandlers = new Set<(s: JobSummary) => void>()
  const scanErrors: ScanError[] = [1, 2, 3].map((i) => ({
    errorId: i, folderId: 3, path: `\\\\NAS\\Archive\\raw\\bad_${i}.mov`, errorCode: "io",
    message: "read failed", attempts: i, lastAt: Date.UTC(2026, 9, 20, 12, i),
  }))

  const jobSummary = (): JobSummary => {
    const d = db()
    let pending = 0
    let failed = 0
    for (const r of d.recs) {
      if (r.status !== "active") continue
      if (r.thumb === "pending") pending++
      else if (r.thumb === "failed") failed++
    }
    const counts = Object.fromEntries(
      JOB_KINDS.map((k) => [k, { pending: 0, running: 0, failed: 0 }]),
    ) as JobSummary["counts"]
    counts.thumb = { pending, running: 0, failed }
    counts.scan.failed = scanErrors.length
    return {
      paused: jobPaused.global,
      pausedKinds: [...jobPaused.kinds],
      counts,
      scan: opts.scanning
        ? { folderId: 1, folderPath: d.folders[0].path, processed: Math.round(count * 0.42), total: count }
        : null,
      totalAssets: d.recs.reduce((n, r) => n + (r.status === "active" ? 1 : 0), 0),
    }
  }
  const emitJobs = () => {
    const s = jobSummary()
    jobHandlers.forEach((h) => h(s))
  }

  const folderView = folderViewOf({ db, settings, now })

  const subfolders = (folderId: number, relPath: string | null): FolderNode[] => {
    const f = db().folders.find((x) => x.folderId === folderId)
    if (!f) throw cmdError("not_found", "folder not found")
    const prefix = relPath ? `${relPath}/` : ""
    const dirCount = new Map<string, number>()
    for (const r of db().recs) {
      if (r.folderId === folderId && r.status === "active") dirCount.set(r.rel, (dirCount.get(r.rel) ?? 0) + 1)
    }
    const children = new Map<string, FolderNode>()
    const allDirs = new Set<string>()
    for (const d of f.dirs) {
      const parts = d.split("/").filter(Boolean)
      for (let i = 1; i <= parts.length; i++) allDirs.add(parts.slice(0, i).join("/"))
    }
    for (const d of allDirs) {
      if (!d.startsWith(prefix) || d === relPath) continue
      const rest = d.slice(prefix.length)
      if (rest.includes("/")) continue
      let assetCount = 0
      for (const [dir, n] of dirCount) if (dir === d || dir.startsWith(`${d}/`)) assetCount += n
      const hasChildren = [...allDirs].some((x) => x.startsWith(`${d}/`))
      children.set(d, { relPath: d, name: rest, hasChildren, assetCount })
    }
    return [...children.values()].sort((a, b) => a.name.localeCompare(b.name))
  }


  return {
    listFolders: async () => (await wait(), db().folders.map(folderView)),
    addFolder: async (path, mediaTypes) => {
      await wait()
      if (noneSelected(mediaTypes)) throw cmdError("invalid_input", "media type required")
      if (!path.trim()) throw cmdError("invalid_input", "path required")
      if (db().folders.some((f) => f.path.toLowerCase() === path.toLowerCase())) throw cmdError("already_registered", "already registered")
      const lower = path.toLowerCase().replace(/\\+$/, "")
      const nested = db().folders.some((f) => {
        const o = f.path.toLowerCase()
        return lower.startsWith(o + "\\") || o.startsWith(lower + "\\")
      })
      if (nested) throw cmdError("nested_folder", "nested folder")
      const f = { folderId: db().folders.length + 1, path, volumeKind: "fixed" as const, volumeLabel: null, offline: false, dirs: [""], mediaTypes }
      db().folders = [...db().folders, f]
      db().version++
      return folderView(f)
    },
    relocateFolder: async (folderId, newPath) => {
      await wait()
      const cur = db().folders.find((f) => f.folderId === folderId)
      if (!cur) throw cmdError("not_found", "not found")
      if (!newPath.trim()) throw cmdError("invalid_input", "path required")
      const lower = newPath.toLowerCase().replace(/\\+$/, "")
      if (db().folders.some((f) => f.path.toLowerCase().replace(/\\+$/, "") === lower)) throw cmdError("already_registered", "already registered")
      const nested = db().folders.some((f) => {
        const o = f.path.toLowerCase()
        return lower.startsWith(o + "\\") || o.startsWith(lower + "\\")
      })
      if (nested) throw cmdError("nested_folder", "nested folder")
      const relocated = folderView(cur).assetCount
      const moved = { ...cur, path: newPath }
      db().folders = db().folders.map((f) => (f.folderId === folderId ? moved : f))
      db().version++
      return { folder: folderView(moved), relocated }
    },
    removeFolder: async (folderId) => {
      await wait()
      db().folders = db().folders.filter((f) => f.folderId !== folderId)
      db().version++
    },
    rescanFolder: async () => wait(),
    setFolderHidden: async (folderId, hidden) => {
      await wait()
      if (!db().folders.some((f) => f.folderId === folderId)) throw cmdError("not_found", "folder not found")
      db().folders = db().folders.map((f) => (f.folderId === folderId ? { ...f, hidden } : f))
      db().version++
    },
    setFolderAlias: async (folderId, alias) => {
      await wait()
      const next = alias === null ? "" : alias.trim()
      if ([...next].length > 64) throw cmdError("invalid_input", "alias too long")
      if (!db().folders.some((f) => f.folderId === folderId)) throw cmdError("not_found", "folder not found")
      db().folders = db().folders.map((f) => (f.folderId === folderId ? { ...f, alias: next === "" ? null : next } : f))
      db().version++
    },
    setFolderMediaTypes: async (folderId, mediaTypes, mode) => {
      await wait()
      if (noneSelected(mediaTypes)) throw cmdError("invalid_input", "media type required")
      if (!db().folders.some((f) => f.folderId === folderId)) throw cmdError("not_found", "folder not found")
      let detached = 0
      if (mode === "purge") {
        for (const r of db().recs) {
          if (r.folderId === folderId && r.status === "active" && !mediaTypes[r.type]) {
            r.status = "deleted"
            detached++
          }
        }
      }
      db().folders = db().folders.map((f) => (f.folderId === folderId ? { ...f, mediaTypes } : f))
      db().version++
      return { detached }
    },
    countFolderAssetsByType: async (folderId) => {
      await wait()
      if (!db().folders.some((f) => f.folderId === folderId)) throw cmdError("not_found", "folder not found")
      const counts = { image: 0, video: 0, audio: 0 }
      for (const r of db().recs) if (r.folderId === folderId && r.status === "active") counts[r.type]++
      return counts
    },
    listSubfolders: async (folderId, relPath) => (await wait(), subfolders(folderId, relPath)),
    jobSummary: async () => (await wait(), jobSummary()),
    pauseJobs: async (kind) => {
      if (kind === null) jobPaused.global = true
      else jobPaused.kinds.add(kind)
      emitJobs()
      return jobSummary()
    },
    resumeJobs: async (kind) => {
      if (kind === null) {
        jobPaused.global = false
        jobPaused.kinds.clear()
      } else jobPaused.kinds.delete(kind)
      emitJobs()
      return jobSummary()
    },
    listScanErrors: async (cursor, limit) => {
      const start = cursor ? Number(cursor) : 0
      if (!Number.isInteger(start) || start < 0 || limit < 1 || limit > 200) throw cmdError("invalid_input", "bad paging")
      const items = scanErrors.slice(start, start + limit)
      return { items, total: scanErrors.length, nextCursor: start + limit < scanErrors.length ? String(start + limit) : null }
    },
    retryScanErrors: async (ids) => (ids === null ? scanErrors.length : ids.length),
    onJobsProgress: async (h) => {
      jobHandlers.add(h)
      return () => void jobHandlers.delete(h)
    },

  }
}
