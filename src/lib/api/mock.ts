import type {
  AssetDetail,
  BackupInfo,
  RecentSearch,
  Settings,
  Tag,
  ThumbSize,
} from "@/lib/ipc-contract"
import { RESUME_MIN_DURATION_CHOICES } from "@/lib/ipc-contract"
import { isValidShortcut } from "@/lib/shortcut"
import type { MockCtx, MockOptions } from "./mock-ctx"
import { createMockAi } from "./mock-ai"
import { generateDb, toSummary, touch, type MockDb, type Rec } from "./mock-data"
import { createMockOps, folderViewOf } from "./mock-ops"
import { createMockRemote } from "./mock-remote"
import { createMockWatch } from "./mock-watch"
import { createMockPlayer } from "./mock-player"
import { createMockProtect } from "./mock-protect"
import { createMockRanges } from "./mock-ranges"
import { createMockResume } from "./mock-resume"
import { SCENE_SENSITIVITIES, createMockScene } from "./mock-scene"
import { createMockVolume } from "./mock-volume"
import { createMockSubtitle } from "./mock-subtitle"
import { createMockReuse } from "./mock-reuse"
import { cmdError, runSearch, runSearchIndex } from "./mock-search"
import { createMockUpdate } from "./mock-update"
import { createMockUsers } from "./mock-users"
import type { UiApi } from "./types"

export type { MockOptions } from "./mock-ctx"

/** 이름 기반 SVG data URL — 앱 스타일이 아니라 합성 이미지 데이터이므로 고정 색을 쓴다. */
export function mockThumbDataUrl(assetId: number): string {
  const hue = (assetId * 47) % 360
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 3"><rect width="4" height="3" fill="hsl(${hue} 45% 55%)"/></svg>`
  return `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`
}

export function createMockApi(opts: MockOptions = {}): UiApi {
  const { count = 100_000, seed = 20261028, now = Date.now, delayMs = 0 } = opts
  let dbCache: MockDb | null = null
  const db = (): MockDb => (dbCache ??= generateDb(count, seed))


  const settings: Settings = {
    dataDir: "C:\\Users\\mock\\AppData\\Roaming\\MAMData",
    rescanIntervalHours: 6,
    maxConcurrentScans: 3,
    authExemptMinutes: 5,
    backup: { enabled: true, intervalHours: 24, keep: 7, dir: null },
    closeToTray: true,
    firstRunCompleted: !opts.firstRun,
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
    indexRail: "always",
    theme: { mode: "system", systemDark: "dim" },
    language: "ko", // 테스트 환경 표시 언어(src/test/setup.ts)와 맞춘다 — 앱 기본값은 en (Rust DEFAULT_LANGUAGE)
    mediaOpenAutoplay: false,
    playRangeKeys: "brackets",
    playRangesOnly: false,
    collectionPlayRangesOnly: false,
    subtitleVisible: true,
    subtitleDefaultLanguage: "ko",
    sceneCutsVisible: true,
    sceneDetectAuto: false,
    sceneDetectSensitivity: "medium",
    playerPlayingStepSec: 5,
    viewerPlacement: "left",
    layout: { detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104 },
    playerVolume: { level: 100, muted: false },
    resumeEnabled: true,
    resumeMinDurationSec: 180,
  }
  const recent: RecentSearch[] = []
  const backups: BackupInfo[] = [...(opts.backups ?? [])]

  const wait = async () => {
    if (delayMs > 0) await new Promise((r) => setTimeout(r, delayMs))
  }
  const rec = (id: number): Rec => {
    const r = db().recs[id - 1]
    if (!r || r.status === "deleted") throw cmdError("not_found", "asset not found")
    return r
  }

  const detail = (id: number): AssetDetail => {
    const d = db()
    const r = rec(id)
    const folder = d.folders.find((f) => f.folderId === r.folderId)
    const dir = r.movedTo ?? [folder?.path ?? "", ...(r.rel ? r.rel.split("/") : [])].join("\\")
    const alive = (x: number) => d.recs[x - 1].status !== "deleted"
    return {
      ...toSummary(r),
      status: r.status,
      path: `${dir}\\${r.name}`,
      folderId: r.folderId,
      volumeLabel: folder?.volumeLabel ?? null,
      codec: r.type === "video" ? "h264" : r.type === "audio" ? "aac" : null,
      audioCodec: r.type === "video" ? "aac" : null,
      fps: r.type === "video" ? 30 : null,
      bitrate: r.type === "image" ? null : 128_000,
      camera: r.type === "image" ? "Mock Camera X1" : null,
      extra: r.type === "image" ? { ISO: "200", Aperture: "f/2.8" } : {},
      playable: r.type === "image" ? r.name.endsWith(".jpg") || r.name.endsWith(".png") : !r.name.endsWith(".mkv"),
      hashState: "done",
      tags: r.tags.map((t) => ({ tagId: t, name: d.tags.get(t) ?? "" })),
      memo: r.memo,
      collections: [...d.collections].filter(([, c]) => c.members.has(id)).map(([collectionId, c]) => ({ collectionId, name: c.name })),
      duplicates: (d.dupGroups.get(r.dup) ?? []).filter((x) => x !== id && alive(x)).map((x) => toSummary(d.recs[x - 1])),
      previousVersions: (d.olderOf.get(id) ?? []).filter(alive).map((x) => toSummary(d.recs[x - 1])),
      newerVersion: r.supersededBy && alive(r.supersededBy) ? toSummary(d.recs[r.supersededBy - 1]) : null,
    }
  }

  const listTags = (): Tag[] => {
    const counts = new Map<number, number>()
    for (const r of db().recs) if (r.status === "active") for (const t of r.tags) counts.set(t, (counts.get(t) ?? 0) + 1)
    return [...db().tags].map(([tagId, name]) => ({ tagId, name, assetCount: counts.get(tagId) ?? 0 }))
  }


  const ctx: MockCtx = { opts, count, now, settings, db, rec, wait }
  const onReset = () => {
    dbCache = null
    recent.length = 0
  }
  const users = createMockUsers({ db, rec, touch, wait, titleEditEnabled: () => settings.titleEditEnabled })
  const ai = createMockAi(ctx)
  const { requireAuth, ...protect } = createMockProtect(ctx, onReset, backups)

  const remote = createMockRemote(ctx)
  return {
    thumbUrl: (assetId: number, _size: ThumbSize) => mockThumbDataUrl(assetId),
    mediaUrl: (assetId: number) => mockThumbDataUrl(assetId),
    ...users,
    ...createMockOps(ctx),
    ...createMockWatch(ctx, folderViewOf(ctx), remote),
    ...remote,
    ...createMockReuse(ctx),
    ...protect,
    ...createMockUpdate(opts),
    ...createMockPlayer(),
    ...createMockRanges({ db, rec, wait }),
    ...createMockResume(ctx),
    ...createMockScene(ctx),
    ...createMockVolume(ctx),
    ...createMockSubtitle({ rec, wait }, opts.subtitles),
    ...ai,

    appPaths: async () => ({
      dataDir: settings.dataDir,
      databasePath: `${settings.dataDir}\\mam.db`,
      backupsDir: `${settings.dataDir}\\backups`,
      logsDir: `${settings.dataDir}\\logs`,
    }),
    getSettings: async () => (await wait(), structuredClone(settings)),
    updateSettings: async (patch) => {
      await wait()
      const { backup, playerVolume, ...rest } = patch
      if (playerVolume !== undefined) {
        const { level, muted } = playerVolume
        if (level !== undefined && (!Number.isInteger(level) || level < 0 || level > 100)) throw cmdError("invalid_input", "playerVolume.level must be an integer 0..100")
        if (muted !== undefined && typeof muted !== "boolean") throw cmdError("invalid_input", "playerVolume.muted must be a boolean")
      }
      if (rest.authExemptMinutes !== undefined && (rest.authExemptMinutes < 0 || rest.authExemptMinutes > 60)) {
        throw cmdError("invalid_input", "authExemptMinutes must be 0..60")
      }
      if (rest.similarMaxResults !== undefined && (!Number.isInteger(rest.similarMaxResults) || rest.similarMaxResults < 6 || rest.similarMaxResults > 200)) {
        throw cmdError("invalid_input", "similarMaxResults must be an integer 6..200")
      }
      if (rest.similarMinSimilarity !== undefined && !(rest.similarMinSimilarity >= 0 && rest.similarMinSimilarity <= 0.99)) {
        throw cmdError("invalid_input", "similarMinSimilarity must be 0..0.99")
      }
      if (rest.galleryPaging !== undefined && rest.galleryPaging !== "infinite" && rest.galleryPaging !== "paged") {
        throw cmdError("invalid_input", "galleryPaging must be infinite|paged")
      }
      if (rest.galleryPageSize !== undefined && ![50, 100, 200].includes(rest.galleryPageSize)) {
        throw cmdError("invalid_input", "galleryPageSize must be 50|100|200")
      }
      if (rest.indexRail !== undefined && !["auto", "always", "off"].includes(rest.indexRail)) {
        throw cmdError("invalid_input", "indexRail must be auto|always|off")
      }
      if (rest.hideShortcut !== undefined && rest.hideShortcut !== null && !isValidShortcut(rest.hideShortcut)) {
        throw cmdError("invalid_input", "hideShortcut must be [Ctrl+][Alt+][Shift+]Key or null")
      }
      if (rest.closeViewerShortcut !== undefined && rest.closeViewerShortcut !== null && !isValidShortcut(rest.closeViewerShortcut)) {
        throw cmdError("invalid_input", "closeViewerShortcut must be [Ctrl+][Alt+][Shift+]Key or null")
      }
      if (rest.maxConcurrentScans !== undefined && (!Number.isInteger(rest.maxConcurrentScans) || rest.maxConcurrentScans < 1 || rest.maxConcurrentScans > 8)) {
        throw cmdError("invalid_input", "maxConcurrentScans must be an integer 1..8")
      }
      if (rest.slideshowImageSeconds !== undefined && (!Number.isInteger(rest.slideshowImageSeconds) || rest.slideshowImageSeconds < 1 || rest.slideshowImageSeconds > 60)) {
        throw cmdError("invalid_input", "slideshowImageSeconds must be an integer 1..60")
      }
      if (rest.playerPlayingStepSec !== undefined && (!Number.isInteger(rest.playerPlayingStepSec) || rest.playerPlayingStepSec < 1 || rest.playerPlayingStepSec > 60)) {
        throw cmdError("invalid_input", "playerPlayingStepSec must be an integer 1..60")
      }
      if (rest.playRangeKeys !== undefined && rest.playRangeKeys !== "brackets" && rest.playRangeKeys !== "premiere") {
        throw cmdError("invalid_input", "playRangeKeys must be brackets|premiere")
      }
      if (rest.viewerPlacement !== undefined && rest.viewerPlacement !== "right" && rest.viewerPlacement !== "left") {
        throw cmdError("invalid_input", "viewerPlacement must be right|left")
      }
      if (rest.subtitleDefaultLanguage !== undefined && rest.subtitleDefaultLanguage !== null && !["ko", "en", "ja", "zh"].includes(rest.subtitleDefaultLanguage)) {
        throw cmdError("invalid_input", "subtitleDefaultLanguage must be ko|en|ja|zh or null")
      }
      if (rest.sceneDetectSensitivity !== undefined && !SCENE_SENSITIVITIES.includes(rest.sceneDetectSensitivity)) {
        throw cmdError("invalid_input", "sceneDetectSensitivity must be low|medium|high")
      }
      for (const k of ["mediaOpenAutoplay", "playRangesOnly", "collectionPlayRangesOnly", "subtitleVisible", "sceneCutsVisible", "sceneDetectAuto", "resumeEnabled"] as const) {
        if (rest[k] !== undefined && typeof rest[k] !== "boolean") throw cmdError("invalid_input", k + " must be a boolean")
      }
      if (rest.resumeMinDurationSec !== undefined && !(RESUME_MIN_DURATION_CHOICES as readonly number[]).includes(rest.resumeMinDurationSec)) {
        throw cmdError("invalid_input", "resumeMinDurationSec must be one of 0|60|180|300|600")
      }
      if (rest.theme !== undefined) {
        const t = rest.theme as Partial<Settings["theme"]> | null
        if (!t || !["system", "light", "gray", "dim", "dark"].includes(t.mode as string) || !["dim", "dark"].includes(t.systemDark as string)) {
          throw cmdError("invalid_input", "theme must be { mode: system|light|gray|dim|dark, systemDark: dim|dark }")
        }
      }
      if (rest.language !== undefined && !/^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$/.test(rest.language)) {
        throw cmdError("invalid_input", "language must be a BCP 47 code")
      }
      // 백엔드와 같은 규칙: 면제 시간을 올리려면 인증(면제 중이거나 0분 1회 허용)이 필요, 내리는 것은 항상 허용
      if (rest.authExemptMinutes !== undefined && rest.authExemptMinutes > settings.authExemptMinutes) requireAuth()
      // 백엔드와 같은 규칙: 고른 폴더 안에 MAMData 를 만든다 (원본 폴더와 섞이지 않도록)
      if (rest.dataDir !== undefined && !/[\\/]MAMData[\\/]?$/i.test(rest.dataDir)) {
        rest.dataDir = `${rest.dataDir.replace(/[\\/]+$/, "")}\\MAMData`
      }
      Object.assign(settings, rest)
      if (playerVolume) Object.assign(settings.playerVolume, playerVolume)
      if (backup) Object.assign(settings.backup, backup)
      return structuredClone(settings)
    },
    backupNow: async () => {
      await wait()
      const b: BackupInfo = { fileName: `manual-${backups.length + 1}.db`, createdAt: now(), sizeBytes: 1_048_576, kind: "manual" }
      backups.unshift(b)
      return b
    },
    listBackups: async () => [...backups],
    openBackupDir: async () => {},
    listLocales: async () => ({ dir: "C:\\Users\\mock\\AppData\\Local\\MAMDesktop\\locales", locales: [] }),
    readLocale: async () => {
      throw cmdError("not_found", "locale not found")
    },
    openLocaleFolder: async () => {},
    hideMainWindow: async () => {},
    setTrayLabels: async () => {},


    searchAssets: async (req) => (await wait(), runSearch(db(), req, ai.semanticReady(), opts.searchGeneration, settings.capturedAtFallback)),
    searchIndex: async (req) => (await wait(), runSearchIndex(db(), req, opts.searchGeneration, settings.capturedAtFallback)),
    getAssetDetail: async (id) => (await wait(), detail(id)),
    listRecentSearches: async () => [...recent],
    recordRecentSearch: async (query, filters) => {
      const q = query.trim()
      if (!q) return
      const i = recent.findIndex((r) => r.query === q)
      if (i >= 0) recent.splice(i, 1)
      recent.unshift({ query: q, filters, usedAt: now() })
      recent.length = Math.min(recent.length, 20)
    },
    clearRecentSearches: async () => void (recent.length = 0),

    listTags: async () => (await wait(), listTags()),

  }
}
