import type { ChangeDetectorInfo, ManagedFolder, RemoteSource, StoredRescanPolicy, WatchOptions } from "@/lib/ipc-contract"
import { WEEKDAYS, isKnownPolicy, normalizeRescanPolicy, validateRescanPolicy } from "@/lib/rescan-policy"
import type { MockCtx } from "./mock-ctx"
import type { MockFolderRec } from "./mock-data"
import { cmdError } from "./mock-search"
import type { UiApi } from "./types"

type WatchApiSlice = Pick<UiApi, "listChangeDetectors" | "setFolderWatchMode" | "setFolderRescanPolicy">

const HOUR_MS = 3_600_000

/** 감지기 옵션 중 모든 감지기가 받는 공통 옵션 (Rust `COMMON_OPTION_FIELDS`) — 목록에는 나오지 않는다 */
export const EXCLUSIVE_OPEN_CHECK = "exclusiveOpenCheck"
const REMOTE_REF = { key: "source", kind: "ref", min: 1, max: Number.MAX_SAFE_INTEGER, default: 0 } as const

/** 이 mock 에 등록된 감지기 — Rust 레지스트리와 같은 순서 */
const DETECTORS: ChangeDetectorInfo[] = [
  { id: "os_events", available: true, reasonKey: null, optionFields: [], experimental: false },
  {
    id: "polling",
    available: true,
    reasonKey: null,
    optionFields: [{ key: "intervalSec", kind: "integer", min: 60, max: 86400, default: 300 }],
    experimental: true,
  },
  { id: "ddp_audit", available: true, reasonKey: null, optionFields: [REMOTE_REF], experimental: true },
  { id: "fanotify_feed", available: true, reasonKey: null, optionFields: [REMOTE_REF], experimental: true },
  { id: "scan_only", available: true, reasonKey: null, optionFields: [], experimental: false },
]
/** 원격 접속 종류 — 참조 옵션이 가리킬 수 있는 종류 */
const REF_KIND = { ddp_audit: "ddp_audit", fanotify_feed: "ses_queue" } as const

const badOptions = () => cmdError("invalid_input", "invalid input: watchOptions")

/** Rust `parse_options_checked` 를 모사한 옵션 검증 — 기본값과 같은 값은 저장하지 않는다 (null = 저장할 것 없음) */
function parseOptions(det: ChangeDetectorInfo | null, raw: WatchOptions | null, sources: RemoteSource[]): WatchOptions | null {
  if (raw === null) return null
  if (typeof raw !== "object" || Array.isArray(raw)) throw badOptions()
  const out: WatchOptions = {}
  for (const [key, value] of Object.entries(raw)) {
    if (key === EXCLUSIVE_OPEN_CHECK) {
      if (typeof value !== "boolean") throw badOptions()
      if (!value) out[key] = false
      continue
    }
    const f = det?.optionFields.find((x) => x.key === key)
    if (!f) throw badOptions()
    if (typeof value !== "number" || !Number.isInteger(value)) throw badOptions()
    if (f.kind === "ref") {
      const want = REF_KIND[det!.id as keyof typeof REF_KIND]
      if (!sources.some((s) => s.sourceId === value && s.kind === want)) throw badOptions()
      out[key] = value
    } else {
      if (value < f.min || value > f.max) throw badOptions()
      if (value !== f.default) out[key] = value
    }
  }
  return Object.keys(out).length > 0 ? out : null
}

/** 가장 가까운 미래의 `HH:MM` (허용 요일 중, 로컬 시각) */
function nextScheduled(from: number, days: readonly string[], at: string): number {
  const [h, m] = at.split(":").map(Number)
  for (let i = 0; i < 9; i++) {
    const d = new Date(from)
    d.setDate(d.getDate() + i)
    d.setHours(h, m, 0, 0)
    const wd = WEEKDAYS[(d.getDay() + 6) % 7]
    if (d.getTime() > from && (days.length === 0 || days.includes(wd))) return d.getTime()
  }
  return from
}

/** `ManagedFolder` 의 변경 감지 6개 필드 — `watchActive` 는 오프라인이면 null, 시드가 정한 폴백 상태는 방식을 바꾸면 지워진다 */
export function mockWatchFields(
  f: MockFolderRec,
  { settingsHours, lastFullScanAt, now }: { settingsHours: number; lastFullScanAt: number; now: number },
): Pick<ManagedFolder, "watchMode" | "watchOptions" | "watchActive" | "watchFallbackKey" | "rescanPolicy" | "nextRescanAt" | "scanStall" | "watchHealth"> {
  const offline = f.offline || f.ejectPrepared === true
  const watchMode = f.watchMode ?? null
  const policy = f.rescanPolicy ?? null
  let nextRescanAt: number | null = null
  if (!offline) {
    if (policy === null) nextRescanAt = lastFullScanAt + settingsHours * HOUR_MS
    else if (isKnownPolicy(policy)) {
      if (policy.trigger.kind === "interval") nextRescanAt = lastFullScanAt + policy.trigger.hours * HOUR_MS
      else if (policy.trigger.kind === "schedule") nextRescanAt = nextScheduled(now, policy.trigger.days, policy.trigger.at)
    }
  }
  return {
    watchMode,
    watchOptions: f.watchOptions ?? null,
    watchActive: offline ? null : (f.watchActive ?? watchMode ?? "os_events"),
    watchFallbackKey: offline ? null : (f.watchFallbackKey ?? null),
    rescanPolicy: policy,
    nextRescanAt,
    // 정상이면 키가 없다 (Rust 의 skip_serializing_if) — 시드가 정한 상태만 싣는다
    ...(f.scanStall && !offline ? { scanStall: f.scanStall } : {}),
    ...(f.watchHealth && !offline ? { watchHealth: f.watchHealth } : {}),
  }
}

/** 변경 감지 방식 · 재스캔 정책 mock — 서버 규칙(검증 필드 경로, 정규화, null = 기본)을 모사한다. */
export function createMockWatch(
  { db, wait }: MockCtx,
  view: (f: MockFolderRec) => ManagedFolder,
  remote: Pick<UiApi, "listRemoteSources">,
): WatchApiSlice {
  const find = (folderId: number): MockFolderRec => {
    const f = db().folders.find((x) => x.folderId === folderId)
    if (!f) throw cmdError("not_found", "folder not found")
    return f
  }
  const patch = (folderId: number, change: Partial<MockFolderRec>): ManagedFolder => {
    db().folders = db().folders.map((f) => (f.folderId === folderId ? { ...f, ...change } : f))
    db().version++
    return view(find(folderId))
  }

  return {
    listChangeDetectors: async (folderId) => {
      await wait()
      find(folderId)
      return structuredClone(DETECTORS)
    },
    setFolderWatchMode: async (folderId, mode, options) => {
      await wait()
      find(folderId)
      if (mode !== null && !DETECTORS.some((d) => d.id === mode)) throw cmdError("invalid_input", "invalid input: watchMode")
      // 자동(모드 null)은 공통 옵션만, 감지기는 선언한 옵션 + 공통 옵션만 받는다 (Rust 검증 규칙)
      const det = mode === null ? null : (DETECTORS.find((d) => d.id === mode) ?? null)
      const stored = parseOptions(det, options, await remote.listRemoteSources())
      return patch(folderId, { watchMode: mode, watchOptions: stored, watchActive: undefined, watchFallbackKey: undefined, watchHealth: undefined })
    },
    setFolderRescanPolicy: async (folderId, policy) => {
      await wait()
      find(folderId)
      if (policy === null) return patch(folderId, { rescanPolicy: null })
      const bad = validateRescanPolicy(policy)
      if (bad !== null) throw cmdError("invalid_input", `invalid input: ${bad}`)
      const stored: StoredRescanPolicy = normalizeRescanPolicy(policy)
      return patch(folderId, { rescanPolicy: stored })
    },
  }
}
