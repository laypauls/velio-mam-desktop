import type {
  AiCardState,
  AssetStatus,
  AssetSummary,
  Availability,
  DerivativeState,
  MediaType,
  MediaTypeSet,
  ScanStall,
  SourceHealth,
  StoredRescanPolicy,
  VolumeKind,
  WatchOptions,
} from "@/lib/ipc-contract"

/** 합성 자산 한 건 (메모리 내부 표현). id = recs 인덱스 + 1. */
export interface Rec {
  id: number
  name: string
  nameLower: string
  type: MediaType
  avail: Availability
  status: AssetStatus
  folderId: number
  /** 관리 폴더 루트 기준 상대 디렉터리 ("" = 루트), `/` 구분 */
  rel: string
  w: number | null
  h: number | null
  dur: number | null
  size: number
  captured: number | null
  modified: number
  fav: boolean
  /** 0..5 (0 = 미평가) */
  rating: number
  thumb: DerivativeState
  /** 카드 AI 표시 (정적 — id 해시로 결정) */
  ai: AiCardState
  rev: number
  tags: number[]
  memo: string
  title: string | null
  /** 완전 중복 그룹 (0 = 없음) */
  dup: number
  /** 이 자산을 대체한 신자산 id (superseded 일 때) */
  supersededBy: number
  /** 이동된 경우 새 디렉터리 (표시용) */
  movedTo: string | null
}

export interface MockFolderRec {
  folderId: number
  path: string
  volumeKind: VolumeKind
  volumeLabel: string | null
  offline: boolean
  dirs: string[]
  hidden?: boolean
  alias?: string | null
  mediaTypes?: MediaTypeSet
  /** 디스크 "꺼내기 준비" 상태 — 켜지면 같은 디스크의 폴더가 모두 `offline` 으로 바뀐다 */
  ejectPrepared?: boolean
  /** 꺼내기 대상 여부 (기본 true) — 내장·네트워크 시드는 false */
  ejectable?: boolean
  /** 변경 감지 방식 (undefined = 자동) */
  watchMode?: string | null
  watchOptions?: WatchOptions | null
  /** 시드가 정한 실제 동작 방식·폴백 사유 (방식을 바꾸면 지워진다) */
  watchActive?: string
  watchFallbackKey?: string
  /** 시드가 정한 신호원 상태·스캔 멈춤 (표시 확인용 — 방식을 바꾸면 상태는 지워진다) */
  watchHealth?: SourceHealth
  scanStall?: ScanStall
  rescanPolicy?: StoredRescanPolicy | null
}

export interface MockDb {
  recs: Rec[]
  folders: MockFolderRec[]
  tags: Map<number, string>
  collections: Map<number, { name: string; createdAt: number; members: Set<number> }>
  dupGroups: Map<number, number[]>
  olderOf: Map<number, number[]>
  nextTagId: number
  nextCollectionId: number
  /** 변경 시 증가 — 검색 캐시 무효화 */
  version: number
}

export const MOCK_TAG_NAMES = [
  "여행", "가족", "회의", "풍경", "인물", "음악", "인터뷰", "드론", "작업물", "백업", "행사", "음식",
]

const FOLDERS: MockFolderRec[] = [
  {
    folderId: 1,
    path: "D:\\Photos",
    volumeKind: "fixed",
    volumeLabel: "Data",
    offline: false,
    ejectable: false,
    dirs: ["", "2024/travel", "2024/family", "2025/busan", "2025/work", "2026/events", "2026/trip"],
  },
  {
    folderId: 2,
    path: "D:\\Videos",
    volumeKind: "fixed",
    volumeLabel: "Data",
    offline: false,
    ejectable: false,
    dirs: ["", "clips", "clips/drone", "interviews", "archive/2023"],
  },
  {
    folderId: 3,
    path: "\\\\NAS\\Archive",
    volumeKind: "network",
    volumeLabel: "NAS",
    offline: true,
    ejectable: false,
    watchActive: "scan_only",
    watchFallbackKey: "start_failed",
    dirs: ["", "raw", "raw/2022", "audio/meetings", "audio/music"],
  },
]

const WORDS = ["busan", "family", "meeting", "sunset", "interview", "market", "report"]
const MEMOS = ["부산 여행 첫날", "클라이언트 검토용", "원본 보관", "편집 예정", "회의 녹음 정리 필요"]
const EXT: Record<MediaType, string[]> = {
  image: ["jpg", "png", "heic", "cr2"],
  video: ["mp4", "mov", "mkv"],
  audio: ["m4a", "wav", "mp3"],
}
const PREFIX: Record<MediaType, string[]> = {
  image: ["IMG", "DSC", "PHOTO"],
  video: ["MOV", "CLIP"],
  audio: ["REC", "TRACK"],
}
const DIMS: [number, number][] = [[4032, 3024], [3024, 4032], [1920, 1080], [3840, 2160], [6000, 4000]]

/** 시각 고정 상수 — 합성 데이터가 실행 시각에 의존하지 않게 한다. */
const T_START = Date.UTC(2022, 0, 1)
const T_END = Date.UTC(2026, 9, 20)

export function mulberry32(seed: number): () => number {
  let a = seed >>> 0
  return () => {
    a = (a + 0x6d2b79f5) >>> 0
    let t = a
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const TYPE_MIX: Record<number, [number, number]> = { 1: [0.9, 0.98], 2: [0.1, 0.95], 3: [0.3, 0.6] }

export function generateDb(count: number, seed: number): MockDb {
  const rnd = mulberry32(seed)
  const pick = <T,>(arr: T[]): T => arr[Math.floor(rnd() * arr.length)]
  const recs: Rec[] = new Array<Rec>(count)
  const tags = new Map(MOCK_TAG_NAMES.map((n, i) => [i + 1, n] as const))

  for (let i = 0; i < count; i++) {
    const r = rnd()
    const folderId = r < 0.55 ? 1 : r < 0.8 ? 2 : 3
    const folder = FOLDERS[folderId - 1]
    const [imgCut, vidCut] = TYPE_MIX[folderId]
    const tr = rnd()
    const type: MediaType = tr < imgCut ? "image" : tr < vidCut ? "video" : "audio"
    const word = rnd() < 0.2 ? `${pick(WORDS)}_` : `${pick(PREFIX[type])}_`
    const name = `${word}${String(i + 1).padStart(6, "0")}.${pick(EXT[type])}`
    const hasDate = rnd() > 0.05
    const captured = hasDate ? Math.floor(T_START + rnd() * (T_END - T_START)) : null
    const modified = (captured ?? T_START) + Math.floor(rnd() * 86_400_000 * 30)
    const dim = type === "audio" ? null : pick(DIMS)
    const size =
      type === "image"
        ? Math.floor(500_000 + rnd() * 12_000_000)
        : type === "video"
          ? Math.floor(20_000_000 + rnd() * 2_000_000_000)
          : Math.floor(1_000_000 + rnd() * 60_000_000)
    const dur =
      type === "image"
        ? null
        : type === "video"
          ? Math.floor(3000 + rnd() * 1_800_000)
          : Math.floor(10_000 + rnd() * 7_200_000)
    const avail: Availability = folder.offline ? "offline" : rnd() < 0.015 ? "missing" : "online"
    const tr2 = rnd()
    const thumb: DerivativeState =
      type === "audio" ? "unsupported" : tr2 < 0.97 ? "done" : tr2 < 0.99 ? "pending" : tr2 < 0.995 ? "failed" : "unsupported"
    const tagIds: number[] = []
    if (rnd() < 0.3) {
      const n = 1 + Math.floor(rnd() * 3)
      for (let k = 0; k < n; k++) {
        const t = 1 + Math.floor(rnd() * MOCK_TAG_NAMES.length)
        if (!tagIds.includes(t)) tagIds.push(t)
      }
    }
    recs[i] = {
      id: i + 1,
      name,
      nameLower: name.toLowerCase(),
      type,
      avail,
      status: rnd() < 0.015 ? "superseded" : "active",
      folderId,
      rel: pick(folder.dirs),
      w: dim?.[0] ?? null,
      h: dim?.[1] ?? null,
      dur,
      size,
      captured,
      modified,
      fav: rnd() < 0.04,
      // 난수열을 건드리지 않도록 id 해시로 결정 (기존 시드 기대값 보존)
      rating: (Math.imul(i + 1, 2654435761) >>> 0) & 1 ? 0 : (Math.imul(i + 1, 2246822519) >>> 0) % 6,
      thumb,
      ai: mockAiState(i + 1),
      rev: 1,
      tags: tagIds,
      memo: rnd() < 0.05 ? pick(MEMOS) : "",
      title: null,
      dup: 0,
      supersededBy: 0,
      movedTo: null,
    }
  }

  const olderOf = new Map<number, number[]>()
  for (const rec of recs) {
    if (rec.status !== "superseded") continue
    let newer = recs[Math.floor(rnd() * count)]
    for (let guard = 0; newer.status === "superseded" && guard < 20; guard++) newer = recs[Math.floor(rnd() * count)]
    if (newer.status === "superseded" || newer.id === rec.id) {
      rec.status = "active"
      continue
    }
    rec.supersededBy = newer.id
    const list = olderOf.get(newer.id) ?? []
    list.push(rec.id)
    olderOf.set(newer.id, list)
  }

  const dupGroups = new Map<number, number[]>()
  const groups = Math.floor(count * 0.015)
  for (let g = 1; g <= groups; g++) {
    const a = recs[Math.floor(rnd() * count)]
    const b = recs[Math.floor(rnd() * count)]
    if (a.id === b.id || a.dup || b.dup || a.status !== "active" || b.status !== "active") continue
    a.dup = b.dup = g
    b.size = a.size
    dupGroups.set(g, [a.id, b.id])
  }

  const collections = new Map<number, { name: string; createdAt: number; members: Set<number> }>()
  ;["포트폴리오", "부산 여행", "클라이언트 A"].forEach((name, i) => {
    const members = new Set<number>()
    for (let k = 0; k < Math.min(200, count / 10); k++) members.add(1 + Math.floor(rnd() * count))
    collections.set(i + 1, { name, createdAt: T_END - i * 86_400_000, members })
  })

  return {
    recs,
    folders: FOLDERS,
    tags,
    collections,
    dupGroups,
    olderOf,
    nextTagId: MOCK_TAG_NAMES.length + 1,
    nextCollectionId: 4,
    version: 1,
  }
}

/** 정적 AI 표시 — 약 35% 완료, 약 3% 실패, 나머지 없음 (난수열을 건드리지 않도록 id 해시) */
function mockAiState(id: number): AiCardState {
  const h = (Math.imul(id, 2654435761) ^ Math.imul(id, 40503)) >>> 0
  const p = h % 100
  return p < 35 ? "done" : p < 38 ? "failed" : "none"
}

export function toSummary(r: Rec): AssetSummary {
  return {
    assetId: r.id,
    fileName: r.name,
    title: r.title,
    mediaType: r.type,
    availability: r.avail,
    width: r.w,
    height: r.h,
    durationMs: r.dur,
    fileSize: r.size,
    capturedAt: r.captured,
    modifiedAt: r.modified,
    favorite: r.fav,
    rating: r.rating,
    thumbState: r.thumb,
    aiState: r.ai,
    revision: r.rev,
  }
}

/** 변경 표시 — revision 증가 + 검색 캐시 무효화. */
export function touch(db: MockDb, r: Rec): void {
  r.rev++
  db.version++
}
