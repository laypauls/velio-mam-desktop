/**
 * P2 UI ↔ Rust 코어 계약 (P2-6~P2-9 화면이 쓰는 command·event).
 *
 * - Rust command 이름은 snake_case, 인자·반환은 serde `rename_all = "camelCase"` 기준이다.
 * - 이 파일은 **계약 정의**다. 실제 호출은 `api/tauri.ts`(invoke), 개발·테스트는 `api/mock.ts` 가 구현한다.
 *   백엔드 command 가 생기면 `api/tauri.ts` 를 `ipc.ts` 의 래퍼로 바꾼다 (계약 타입은 그대로).
 * - 모든 command 는 실패 시 `CommandError` 로 reject 한다. 내부 경로·키는 message 에 넣지 않는다.
 * - 시각은 UTC Unix ms (number). id 는 SQLite INTEGER (number — 2^53 미만).
 */

import type { AiApi, AiErrorCode, SearchMode } from "./ipc-contract-ai"
import type { PlayerApi, PlayerErrorCode } from "./ipc-contract-player"
import type { PluginErrorCode } from "./ipc-contract-plugin"
import type { PlayRangeKeys, RangesApi } from "./ipc-contract-ranges"
export type { PlayRange, PlayRangeKeys, PlayRangeSummary, RangesApi } from "./ipc-contract-ranges"
import type { ResumeApi } from "./ipc-contract-resume"
export type { ResumeApi, ResumePosition, ResumeSaveOutcome, ResumeStats } from "./ipc-contract-resume"
export { RESUME_MIN_DURATION_CHOICES } from "./ipc-contract-resume"
import type { UserFieldFilter } from "./ipc-contract-metadata"
export type {
  AssetMetadataValue,
  MetadataField,
  MetadataFieldOption,
  MetadataPatch,
  MetadataValue,
  MetadataValueType,
  UserFieldFilter,
} from "./ipc-contract-metadata"
import type { LocaleApi } from "./ipc-contract-locale"
export type { LocaleApi, LocaleFileError, LocaleInfo, LocaleList, LocaleResource } from "./ipc-contract-locale"
import type { SceneApi, SceneSensitivity } from "./ipc-contract-scene"
export type {
  CutSource,
  SceneApi,
  SceneBackfillResult,
  SceneCut,
  SceneCuts,
  SceneDetectOptions,
  SceneFailure,
  SceneRequestResult,
  SceneSensitivity,
  SceneState,
} from "./ipc-contract-scene"
import type { VolumeApi } from "./ipc-contract-volume"
export type {
  EjectPrepared,
  FolderRemovalImpact,
  FolderRemovalProgress,
  IgnoredVolume,
  KnownVolume,
  KnownVolumeKind,
  PendingVolume,
  PendingVolumeKind,
  PendingVolumesPayload,
  VolumeApi,
} from "./ipc-contract-volume"
export { PENDING_VOLUMES_EVENT } from "./ipc-contract-volume"
import type { RemoteApi } from "./ipc-contract-remote"
export type { PathMapping, RemoteApi, RemoteAuthKind, RemoteSource, RemoteSourceInput, RemoteSourceKind, RemoteTestResult } from "./ipc-contract-remote"
import type { ScanStall, SourceHealth, StoredRescanPolicy, WatchApi, WatchOptions } from "./ipc-contract-watch"
export type {
  ChangeDetectorInfo,
  DetectorOptionField,
  KnownTriggerKind,
  RescanPolicy,
  RescanTrigger,
  ScanStall,
  SourceHealth,
  StoredRescanPolicy,
  TimeWindow,
  WatchApi,
  WatchOptions,
  Weekday,
} from "./ipc-contract-watch"
export { KNOWN_TRIGGER_KINDS } from "./ipc-contract-watch"
import type { SubtitleApi, SubtitleLanguage } from "./ipc-contract-subtitle"
export type { SubtitleApi, SubtitleCue, SubtitleFormat, SubtitleLanguage, SubtitleTrack } from "./ipc-contract-subtitle"
import type { UpdateCheck, UpdateError, UpdateStatePayload } from "./ipc"

export type { UpdateCheck, UpdateError, UpdateStatePayload }

// ─── 공통 ────────────────────────────────────────────────────────────────────

export type ErrorCode =
  | "invalid_input"
  | "not_found"
  | "already_registered"
  | "nested_folder"
  /** 폴더 이전 요청 시 그 폴더의 스캔이 실행 중이다 — 일시정지 후 다시 시도 */
  | "scan_running"
  | "offline"
  | "auth_required"
  | "password_incorrect"
  /** 연속 5회 실패 후 30초 동안 — 이 시간에는 맞는 비밀번호도 거부된다 (P2-9 D2) */
  | "too_many_attempts"
  | "password_not_set"
  | "password_already_set"
  | "disk_full"
  | "io"
  | "internal"
  /** 시작 직후 색인 재구축 중 — 잠시 뒤 재시도하면 된다 (P4 후속) */
  | "index_not_ready"
  /** 백업 파일이 없거나 손상됨 — 복원 요청 단계에서 거부 (대상 DB 불변) */
  | "backup_invalid"
  /** 백업의 스키마 버전이 현재 앱보다 높음 */
  | "backup_too_new"
  /** 폴더 제거 직전 DB 백업 실패 — 아무것도 바꾸지 않았다 */
  | "backup_failed"
  /** 관리 폴더가 남은 디스크의 기록 지우기 거부 */
  | "volume_has_folders"
  /** 같은 범위의 폴더 제거가 진행 중 */
  | "busy"
  // P3 AI (ipc-contract-ai.ts)
  | AiErrorCode
  // 내장 플레이어 (ipc-contract-player.ts)
    // 플러그인 (ipc-contract-plugin.ts)
  | PluginErrorCode
| PlayerErrorCode

export interface CommandError {
  code: ErrorCode
  message: string
}

/** 페이지 단위 조회 결과. `nextCursor` 가 null 이면 마지막 페이지. */
export interface Page<T> {
  items: T[]
  total: number
  nextCursor: string | null
}

export const MAX_PAGE_LIMIT = 200

export type MediaType = "image" | "video" | "audio"

/** 자산 접근 상태 — online: 정상, offline: 볼륨 연결 끊김, missing: 볼륨은 연결됐는데 파일 없음. */
export type Availability = "online" | "offline" | "missing"

export type AssetStatus = "active" | "superseded" | "deleted"

export type DerivativeState = "pending" | "done" | "failed" | "unsupported"

// ─── 저장소·설정 (P2-1, P2-9) ────────────────────────────────────────────────

/** `app_paths` */
export interface AppPaths {
  dataDir: string
  databasePath: string
  backupsDir: string
  logsDir: string
}

export interface BackupSettings {
  enabled: boolean
  /** PL9 기본 24 */
  intervalHours: number
  /** PL9 기본 7 */
  keep: number
  /** null = 기본 `MAMData/backups` */
  dir: string | null
}

/** 카드 크기 단계 — 목표 폭은 프론트 `CARD_SIZES` */
export type CardSize = "xs" | "s" | "m" | "l" | "xl"

/** 갤러리 카드 표시 옵션 (설정 키 `gallery_card_display`, 값은 JSON 객체 1개). 프론트는 항상 전체 객체를 보낸다 */
export interface GalleryCardDisplay {
  /** 썸네일 아래 정보 영역 항목 */
  info: { typeIcon: boolean; name: boolean; ext: boolean; resolution: boolean; size: boolean; date: boolean }
  /** 날짜 기준 — captured: 촬영일 우선(없으면 수정일), modified: 수정일 */
  dateBy: "captured" | "modified"
  /** 선택 체크 표시 방식 */
  check: "hover" | "always" | "hidden"
  favorite: boolean
  /** 카드 하단 별점 표시 (기본 true) */
  rating: boolean
  duration: boolean
  quickActions: boolean
  /** 카드 크기 단계 — 없으면 m */
  cardSize: CardSize
}

/** 정렬 드롭다운에 노출할 항목 — 관련도(relevance)는 항상 노출되므로 여기 없다. 프론트는 항상 전체 객체를 보낸다 */
export interface SortFieldsEnabled {
  capturedAt: boolean
  modifiedAt: boolean
  name: boolean
  size: boolean
  duration: boolean
  rating: boolean
}

/** 촬영일이 없는 자산의 날짜 대체 — none: 빈값, modified: 파일 수정일 */
export type CapturedAtFallback = "none" | "modified"

/** 콘텐츠 더블클릭(Enter 포함) 동작 */
export type DoubleClickAction = "preview" | "openSystem"

/** 갤러리 목록 탐색 방식 — 무한 스크롤 / 페이지 나눔 */
export type GalleryPaging = "infinite" | "paged"

/** 목차 레일 표시 — 자동(목록이 길 때만) / 항상 / 끔 */
export type IndexRailMode = "auto" | "always" | "off"

/** 플레이어 패널 위치: right = 갤러리 오른쪽(기본), left = 창 맨 왼쪽(사이드바 앞) */
export type ViewerPlacement = "right" | "left"

/** 앱 테마 선택 — system: Windows 앱 모드를 따름 */
export type ThemeMode = "system" | "light" | "gray" | "dim" | "dark"

/** 시스템이 다크일 때 쓸 테마 */
export type ThemeSystemDark = "dim" | "dark"

/** 앱 테마 설정 (설정 키 `theme`, 값은 JSON 객체 1개). 프론트는 항상 두 필드를 모두 보낸다 */
export interface ThemeSetting {
  mode: ThemeMode
  systemDark: ThemeSystemDark
}

/** 왼쪽 레일에서 마지막으로 연 섹션 */
export type SidebarSection = "library" | "rating" | "folders" | "tags" | "collections"

/** 레이아웃 기억 (설정 키 `layout`, 값은 JSON 객체 1개). 프론트는 항상 여섯 필드를 모두 보낸다 */
export interface LayoutSettings {
  detailOpen: boolean
  sidebarOpen: boolean
  /** 콘텐츠 보기 폭(px, 최소 320, 기본 520) */
  viewerWidth: number
  detailTab: "info" | "ai" | "similar"
  /** 필름스트립 높이(px, 72~160, 기본 104). Rust 가 범위 밖 값을 clamp 한다 */
  filmstripHeight: number
  /** 왼쪽 레일 마지막 섹션(기본 "library"). Rust 가 알 수 없는 값을 "library" 로 보정한다 */
  sidebarSection: SidebarSection
}

/** 플레이어 음량 기억 (설정 키 `playerVolume`, 값은 JSON 객체 1개). 음소거와 level 0 은 구분된다 */
export interface PlayerVolumeSettings {
  /** 0~100 정수, 기본 100 */
  level: number
  /** 기본 false — 해제하면 level 로 복귀 */
  muted: boolean
}

/** `get_settings` */
export interface Settings {
  /** 다음 실행부터 적용되는 관리 데이터 위치 (PL11). 로컬 드라이브만 허용 */
  dataDir: string
  /** 주기 재스캔 간격 (P2-2 D6, 기본 6) */
  rescanIntervalHours: number
  /** 동시에 도는 스캔의 전체 상한 (정수 1~8, 기본 3) — 같은 볼륨의 스캔은 항상 하나씩이라 볼륨이 다를 때만 겹친다 */
  maxConcurrentScans: number
  /** 비밀번호 인증 면제 시간 (PL1, 0~60, 기본 5) */
  authExemptMinutes: number
  backup: BackupSettings
  /** 창 닫기 시 트레이로 (요구사항 §2.4, 기본 true) */
  closeToTray: boolean
  /** 최초 실행 안내 완료 여부 */
  firstRunCompleted: boolean
  /** 갤러리 카드 표시 옵션 (기본: 정보 항목 전부 켬) */
  galleryCardDisplay: GalleryCardDisplay
  /** 콘텐츠 제목 편집 허용 (기본 false — 꺼짐 = 파일명 표시, 저장한 제목은 보존) */
  titleEditEnabled: boolean
  /** 원본 이동·삭제 동작을 UI 에 노출 (기본 false — 표시 전용, 백엔드 비밀번호 인증은 그대로) */
  originalActionsEnabled: boolean
  /** 사이드바 별점 메뉴 표시 (기본 true) */
  sidebarRatingMenu: boolean
  /** 동영상을 내장 플레이어로 재생 (기본 true — 끄면 WebView 재생 가능 영상은 앱 기본 재생) */
  videoPlayerBuiltIn: boolean
  /** 재생 정지 후 숨기기 단축키 (기본 "Escape", null = 사용 안 함). 형식: [Ctrl+][Alt+][Shift+]키 — 키는 KeyboardEvent.key(한 글자는 대문자) */
  hideShortcut: string | null
  /** 플레이어 패널 닫기 단축키 (기본 "Q", null = 사용 안 함). 형식은 hideShortcut 과 같다 */
  closeViewerShortcut: string | null
  /** 연속 재생 이미지 표시 시간(초, 1~60, 기본 5) */
  slideshowImageSeconds: number
  /** 정렬 항목별 노출 (기본 모두 true) */
  sortFieldsEnabled: SortFieldsEnabled
  /** 더블클릭·Enter 동작 (기본 "preview") */
  doubleClickAction: DoubleClickAction
  /** 촬영일이 없을 때 정렬·목차·필터·표시에 쓸 날짜 (기본 "modified") */
  capturedAtFallback: CapturedAtFallback
  /** 유사 미디어 최대 표시 개수 (정수 6~200, 기본 24) */
  similarMaxResults: number
  /** 유사 미디어 최소 유사도 (0~0.99, 기본 0.75, 0 = 제한 없음) */
  similarMinSimilarity: number
  /** 갤러리 탐색 방식 (기본 "infinite") */
  galleryPaging: GalleryPaging
  /** 페이지 모드 한 페이지 크기 (50|100|200 중 하나, 기본 100) */
  galleryPageSize: 50 | 100 | 200
  /** 목차 레일 (기본 "auto") */
  indexRail: IndexRailMode
  /** 앱 테마 (기본 { mode: "system", systemDark: "dim" }) */
  theme: ThemeSetting
  /** UI 표시 언어 코드 (기본 "en"). AI 출력 언어와 별개 */
  language: string
  /** 동영상·음성을 열 때 자동 재생 (기본 false = 정지 상태로 열기, 컬렉션 연속 재생은 항상 자동 재생) */
  mediaOpenAutoplay: boolean
  /** 재생 구간 단축키 프리셋 (기본 "brackets") */
  playRangeKeys: PlayRangeKeys
  /** 단독 재생에서 구간만 재생 (기본 false) */
  playRangesOnly: boolean
  /** 컬렉션 연속 재생에서 구간만 재생 (기본 false) */
  collectionPlayRangesOnly: boolean
  /** 동영상 자막 표시 (기본 true) — 플레이어 CC 켜기/끄기와 같은 값 */
  subtitleVisible: boolean
  /** 자막 기본 언어 (기본 "ko", null = 자막 끄고 시작) */
  subtitleDefaultLanguage: SubtitleLanguage | null
  /** 탐색 바에 장면전환점 표시 (기본 true) */
  sceneCutsVisible: boolean
  /** 새로 색인되는 동영상의 장면전환점 자동 추출 (기본 false — CPU 를 많이 쓴다) */
  sceneDetectAuto: boolean
  /** 장면 검출 민감도 (기본 "medium") */
  sceneDetectSensitivity: SceneSensitivity
  /** 플레이어 이동 버튼의 재생 중 이동 폭(초, 정수 1~60, 기본 5). 정지 중에는 1프레임 */
  playerPlayingStepSec: number
  /** 플레이어 패널(콘텐츠 보기 영역) 위치 (기본 "right" = 갤러리 오른쪽, "left" = 창 맨 왼쪽·사이드바 앞) */
  viewerPlacement: ViewerPlacement
  /** 상세 패널·사이드바 열림, 콘텐츠 보기 폭, 상세 탭 (기본 true/true/520/"info") */
  layout: LayoutSettings
  /** 플레이어 음량·음소거 (기본 { level: 100, muted: false }, level 0~100 정수). 두 재생 경로가 공유한다 */
  playerVolume: PlayerVolumeSettings
  /** 동영상·음성 이어보기 사용 (기본 true). 끄면 저장·복원을 모두 멈추고 기존 기록은 유지한다 */
  resumeEnabled: boolean
  /** 이어보기 최소 콘텐츠 길이(초, 허용 0·60·180·300·600, 기본 180) — 이보다 짧은 콘텐츠는 기억하지 않는다 */
  resumeMinDurationSec: number
}

export type SettingsPatch = Partial<Omit<Settings, "backup" | "playerVolume">> & {
  backup?: Partial<BackupSettings>
  playerVolume?: Partial<PlayerVolumeSettings>
}

export type BackupKind = "auto" | "manual" | "pre_migrate" | "pre_reset" | "pre_restore" | "pre_remove"

/** `restore_backup` 입력 — 비밀번호가 설정돼 있으면 `password` 필수 (면제 시간과 무관) */
export interface RestoreBackupInput {
  fileName: string
  password?: string
}

/** `take_startup_notice` — 기동 때 복원/자동 복구가 일어났다는 1회성 알림. 한 번 읽으면 지워진다. */
export type StartupNotice =
  | { kind: "restored"; backupFile: string; createdAt: number }
  | {
      kind: "recovered"
      /** null = 쓸 수 있는 백업이 없어 빈 DB 로 시작 */
      backupFile: string | null
      createdAt: number | null
      /** 보관한 손상 DB 경로 */
      corruptFile: string
      /** 자동 복구에서는 외부 AI 동의가 철회 상태로 시작한다 */
      consentReset: boolean
    }

/** `list_backups` 항목 */
export interface BackupInfo {
  fileName: string
  createdAt: number
  sizeBytes: number
  kind: BackupKind
}

// ─── 관리 폴더·작업 (P2-2) ───────────────────────────────────────────────────

export type VolumeKind = "fixed" | "removable" | "network"

export type FolderScanState = "idle" | "scanning" | "paused" | "failed"

/** 관리 폴더별 스캔 대상 유형 (최소 1개 true) */
export interface MediaTypeSet {
  image: boolean
  video: boolean
  audio: boolean
}

/** 유형별 활성 자산 수 */
export interface MediaTypeCounts {
  image: number
  video: number
  audio: number
}

/** 대상 유형 변경 시 제외되는 유형의 기존 항목 처리 */
export type MediaTypeChangeMode = "keep" | "purge"

/** set_folder_media_types 결과 */
export interface SetFolderMediaTypesResult {
  /** purge 로 라이브러리에서 제거(detach)된 위치 수 (keep 이면 0) */
  detached: number
}

/** `list_folders` 항목 */
export interface ManagedFolder {
  folderId: number
  /** 표시용 절대 경로 (현재 드라이브 문자 기준) */
  path: string
  volumeKind: VolumeKind
  volumeLabel: string | null
  /** 폴더가 있는 볼륨의 연결 상태 */
  availability: "online" | "offline"
  assetCount: number
  lastFullScanAt: number | null
  scanState: FolderScanState
  /** 보기에서 숨김 (스캔·분석은 계속) */
  hidden: boolean
  /** 사용자 지정 표시 이름 (최대 64자, 없으면 null → 경로 표시) */
  alias: string | null
  /** 스캔 대상 유형 */
  mediaTypes: MediaTypeSet
  /** 이 폴더의 디스크가 "꺼내기 준비" 상태 (이때 availability 는 "offline") */
  ejectPrepared: boolean
  /** 꺼내기 대상으로 보일 수 있는 폴더 — 내장 디스크·시스템 볼륨·MAMData 볼륨·네트워크는 false */
  ejectable: boolean
  /** 변경 감지 방식 ID (null = 자동) */
  watchMode: string | null
  /** 감지기별 옵션 (null = 기본값) */
  watchOptions: WatchOptions | null
  /** 실제 동작 중인 감지 방식 — 오프라인·꺼내기 보류 등으로 감시 중이 아니면 null */
  watchActive: string | null
  /** 선택한 방식 대신 다른 방식으로 동작하는 사유 키 (`watch.reason.<key>`) */
  watchFallbackKey: string | null
  /** 재스캔 정책 저장값 (null = 전역 기본 `rescanIntervalHours`) */
  rescanPolicy: StoredRescanPolicy | null
  /** 다음 자동 재스캔 예정 (UTC ms, 수동·오프라인이면 null) */
  nextRescanAt: number | null
  /** 스캔이 3분 넘게 한 곳에서 멈춰 있을 때만 존재 (정상이면 키 없음) — 붙들린 경로와 그 시작 시각 */
  scanStall?: ScanStall
  /** 변경 신호원 상태 — 마지막 신호를 받았거나 저하 중일 때만 존재 (정상·신호 없음이면 키 없음) */
  watchHealth?: SourceHealth
}

/** `relocate_folder` 결과 */
export interface RelocateFolderResult {
  folder: ManagedFolder
  /** 새 경로로 옮겨 연결한 위치 수 */
  relocated: number
}

/** `list_subfolders` 항목 — 폴더 탐색 트리 (지연 로딩) */
export interface FolderNode {
  /** 관리 폴더 루트 기준 상대 경로, `/` 구분. 루트 바로 아래면 이름과 같다 */
  relPath: string
  name: string
  hasChildren: boolean
  /** 하위 폴더 포함 자산 수 */
  assetCount: number
}

export type JobKind = "scan" | "meta" | "thumb" | "hash" | "index" | "ai" | "scene" | "plugin"

export interface JobCounts {
  pending: number
  running: number
  failed: number
}

export interface ScanProgress {
  folderId: number
  folderPath: string
  processed: number
  /** 전체 파일 수를 아직 모르면 null */
  total: number | null
}

/** `job_summary` 반환 및 `jobs://progress` 이벤트 페이로드 (최대 4Hz) */
export interface JobSummary {
  /** 전역 일시정지 */
  paused: boolean
  pausedKinds: JobKind[]
  counts: Record<JobKind, JobCounts>
  scan: ScanProgress | null
  /** 전체 활성 자산 수 */
  totalAssets: number
}

export const JOBS_PROGRESS_EVENT = "jobs://progress"

/** `list_scan_errors` 항목 */
export interface ScanError {
  errorId: number
  folderId: number
  path: string
  errorCode: string
  message: string
  attempts: number
  lastAt: number
}

// ─── 검색·조회 (P2-5) ───────────────────────────────────────────────────────

export type SortField =
  | "relevance"
  | "capturedAt"
  | "modifiedAt"
  | "name"
  | "size"
  | "duration"
  | "rating"
  /** 컬렉션 안 순서 — `filters.collectionId` 가 있을 때만 유효(없으면 invalid_input). 프론트는 asc 로 고정 */
  | "collectionOrder"
export type SortDirection = "asc" | "desc"

export interface SortSpec {
  field: SortField
  direction: SortDirection
}

export interface FolderScope {
  folderId: number
  /** null = 관리 폴더 루트 */
  relPath: string | null
  /** 하위 폴더 포함 */
  recursive: boolean
}

export interface SearchFilters {
  /** 빈 배열 = 전체 유형 */
  mediaTypes: MediaType[]
  /** PL5 기본: 오프라인 표시(true), 파일 없음 숨김(false) */
  includeOffline: boolean
  includeMissing: boolean
  capturedFrom: number | null
  capturedTo: number | null
  sizeMin: number | null
  sizeMax: number | null
  durationMinMs: number | null
  durationMaxMs: number | null
  folder: FolderScope | null
  /** 별점 범위(포함). min==max 정확히 N점, 0 = 미평가. null = 제한 없음 */
  ratingMin: number | null
  ratingMax: number | null
  favoritesOnly: boolean
  collectionId: number | null
  /** 모두 가진 자산 (AND) */
  tagIds: number[]
  /** 사용자 정의 필드 조건 (AND). 생략 = 없음 — 서버는 빈 값이면 기록용 JSON 에도 싣지 않는다 */
  userFields?: UserFieldFilter[]
}

/** `search_assets` 인자. query 가 빈 문자열이면 필터·정렬만으로 조회(탐색) */
export interface SearchRequest {
  query: string
  filters: SearchFilters
  sort: SortSpec
  /** 1 ~ MAX_PAGE_LIMIT */
  limit: number
  cursor: string | null
  /** 생략 시 "keyword" (P3 의미 검색: "hybrid") */
  mode?: SearchMode
  /**
   * 생략 시 "forward". "backward" 는 `cursor`(= 페이지의 `prevCursor`) 바로 앞 `limit` 건을 표시 순서 그대로 돌려준다.
   * cursor 없이 "backward" 는 `invalid_input`.
   */
  direction?: PageDirection
  /** 전역 0-기준 위치로 바로 가기. `cursor` 와 동시 지정 불가(`invalid_input`). `>= total` 이면 빈 페이지 (`startOffset = total`) */
  at?: number
}

export type PageDirection = "forward" | "backward"

/** 카드 AI 표시 — none: AI 결과 없음 / done: 결과 있음 / failed: 결과 없고 embedding 외 단계 실패 */
export type AiCardState = "none" | "done" | "failed"

/** 갤러리·목록 한 칸 */
export interface AssetSummary {
  assetId: number
  fileName: string
  /** 사용자가 지정한 제목 (저장값 그대로 — 설정이 꺼져 있어도 값은 온다). 표시는 displayName() */
  title: string | null
  mediaType: MediaType
  availability: Availability
  width: number | null
  height: number | null
  durationMs: number | null
  fileSize: number
  capturedAt: number | null
  modifiedAt: number
  favorite: boolean
  /** 0..5 (0 = 미평가) */
  rating: number
  thumbState: DerivativeState
  /** 카드 AI 표시용 집계 */
  aiState: AiCardState
  revision: number
}

/** `search_assets` 반환 */
export interface SearchPage extends Page<AssetSummary> {
  /** 질의가 있을 때 total 은 추정치일 수 있다 */
  totalIsEstimate: boolean
  tookMs: number
  /** hybrid 요청을 keyword 로 처리했다 (모델 미준비 등) */
  degraded?: boolean
  /** 이 페이지 앞에 항목이 있으면 `direction: "backward"` 로 쓰는 커서. 맨 앞·빈 페이지는 null */
  prevCursor: string | null
  /** `items[0]` 의 전역 0-기준 위치 (커서 페이지도 계산해 준다) */
  startOffset: number
  /** 색인 세대(JS 안전 정수). `SearchIndex.generation` 과 다르면 블록 캐시·구간 정보를 버린다 */
  generation: number
}

/** `search_index` 인자 — `search_assets` 와 같은 질의·필터·정렬 (limit·cursor 없음) */
export interface SearchIndexRequest {
  query: string
  filters: SearchFilters
  sort: SortSpec
  mode?: SearchMode
}

/**
 * 구간 종류. month: 촬영일·수정일 정렬(OS 로컬 시간대의 월, 프론트 `monthKey` 와 같은 구간),
 * initial: 이름·제목 정렬(첫 글자), none: 그 밖의 정렬(크기·길이·관련도) — 단일 구간 "all"
 */
export type SearchIndexKind = "month" | "initial" | "none"

export interface SearchIndexSection {
  /**
   * month: "YYYY-MM" | "none"(날짜 없음, 방향과 무관하게 맨 뒤).
   * initial: "sym"(a 앞 전부) · "a"~"z" · "etc"(z 뒤 ~ 가 앞) · "ㄱ"~"ㅎ"(초성, ㄲ→ㄱ 합침) · "etc_end"(힣 뒤).
   * none: "all"
   */
  key: string
  /** 항상 null — 표시 문구는 프론트가 번역 키/포맷으로 만든다 */
  label: string | null
  /** 1 이상 (빈 구간은 오지 않는다) */
  count: number
}

/** `search_index` 반환 — 정렬 순서(내림차순이면 뒤집힘)대로 구간별 개수. Σcount = total */
export interface SearchIndex {
  total: number
  generation: number
  kind: SearchIndexKind
  sections: SearchIndexSection[]
}

export interface TagRef {
  tagId: number
  name: string
}

export interface CollectionRef {
  collectionId: number
  name: string
}

/** `get_asset_detail` 반환 */
export interface AssetDetail extends AssetSummary {
  status: AssetStatus
  /** 표시용 전체 경로. 오프라인이면 마지막으로 알려진 경로 */
  path: string
  folderId: number | null
  volumeLabel: string | null
  codec: string | null
  audioCodec: string | null
  fps: number | null
  bitrate: number | null
  camera: string | null
  /** 추가 메타데이터 (표시용 키-값, 키는 i18n 키가 아닌 원문) */
  extra: Record<string, string>
  /** WebView2 에서 직접 재생 가능한지 (P1-5) */
  playable: boolean
  hashState: "pending" | "done" | "failed"
  tags: TagRef[]
  memo: string
  collections: CollectionRef[]
  /** 내용 해시가 같은 다른 자산 (완전 중복) */
  duplicates: AssetSummary[]
  /** 이 자산이 대체한 구자산 (PL6, superseded) */
  previousVersions: AssetSummary[]
  /** 이 자산을 대체한 신자산 (이 자산이 superseded 일 때) */
  newerVersion: AssetSummary | null
}

export interface RecentSearch {
  query: string
  filters: SearchFilters
  usedAt: number
}

// ─── 사용자 데이터 (P2-7) ───────────────────────────────────────────────────

export interface Tag extends TagRef {
  assetCount: number
}

export interface Collection extends CollectionRef {
  assetCount: number
  createdAt: number
}

/** 컬렉션 안 이동 위치 — 선택 항목을 상대 순서 유지한 채 한 덩어리로 옮긴다 */
export type ReorderTarget = { kind: "before"; assetId: number } | { kind: "start" } | { kind: "end" }

/** `collection_membership` 항목 — count 가 0 인 컬렉션은 응답에 없다 */
export interface CollectionMembership {
  collectionId: number
  count: number
}

/** `collection_play_queue` 항목 — 컬렉션 순서대로, 오프라인·파일 없음 포함 */
export interface PlayQueueItem {
  assetId: number
  mediaType: MediaType
  availability: Availability
  fileName: string
  title: string | null
  durationMs: number | null
}

export interface UpdateTagsInput {
  assetIds: number[]
  /** 태그 이름 — 없으면 만든다 (이름 비교는 대소문자·NFC 정규화 기준) */
  add: string[]
  remove: number[]
}

// ─── 재활용 (P2-8) ──────────────────────────────────────────────────────────

export type ConflictPolicy = "skip" | "rename" | "overwrite"

/** 저장 대화상자 확장자 필터 (`pick_save_file`). extensions 는 점 없는 영숫자. */
export interface SaveFileFilter {
  name: string
  extensions: string[]
}

export interface ExportInput {
  assetIds: number[]
  destDir: string
  conflict: ConflictPolicy
  /** 대상 폴더에 메타데이터 CSV(mam-export.csv)를 함께 쓴다 */
  includeManifest: boolean
}

export interface TaskStarted {
  taskId: string
}

/** `export://progress` 이벤트 페이로드 */
export interface ExportProgress {
  taskId: string
  done: number
  total: number
  failed: number
  skipped: number
  finished: boolean
}

export const EXPORT_PROGRESS_EVENT = "export://progress"

// ─── 보호·운영 (P2-9) ───────────────────────────────────────────────────────

/** `get_protection_status` */
export interface ProtectionStatus {
  passwordSet: boolean
  /** 인증 면제 만료 시각. null = 인증 안 됨 */
  authValidUntil: number | null
}

/**
 * 원본 삭제·이동의 자산별 실패 사유 코드 (`OriginalsResult.failed[].message`). 백엔드는 경로·DB 원문 대신
 * 이 고정 코드만 보낸다 (보안 리뷰 #7). 목록에 없는 코드는 UI 가 "기타" 로 보여 준다.
 */
export const ORIGINALS_FAILURE_CODES = [
  "file_missing",
  "offline",
  "changed_since_scan",
  "destination_exists",
  "destination_tracked",
  "already_in_destination",
  "recycle_failed",
  "no_recycle_bin",
  "moved_db_pending",
  "copy_verify_failed",
  "invalid_input",
  "not_found",
  "io",
  "internal",
] as const

export type OriginalsFailureCode = (typeof ORIGINALS_FAILURE_CODES)[number]

export interface OriginalsResult {
  succeeded: number
  /** message = OriginalsFailureCode (알 수 없는 코드가 올 수 있어 string) */
  failed: { assetId: number; message: string }[]
}

export interface ResetInput {
  /** 비밀번호를 아는 경우 */
  password: string | null
  /** 비밀번호 분실 경로 — 인증 없이 관리 데이터 초기화 (원본 무관, PL3) */
  forgotPassword: boolean
}

/** 트레이 우클릭 메뉴 문구 — 표시 언어로 번역·보간이 끝난 텍스트(`aiWaiting` 포함). */
export interface TrayLabels {
  open: string
  pauseJobs: string
  resumeJobs: string
  quit: string
  aiWaiting: string
}

// ─── API 인터페이스 ─────────────────────────────────────────────────────────

/** UI 가 쓰는 코어 API. 주석은 Rust command 이름과 인자 이름이다. */
export interface MamApi extends AiApi, LocaleApi, PlayerApi, RangesApi, RemoteApi, ResumeApi, SceneApi, SubtitleApi, VolumeApi, WatchApi {
  // P2-1 / P2-9 설정·백업
  appPaths(): Promise<AppPaths> // app_paths
  getSettings(): Promise<Settings> // get_settings
  updateSettings(patch: SettingsPatch): Promise<Settings> // update_settings { patch }
  backupNow(): Promise<BackupInfo> // backup_now
  listBackups(): Promise<BackupInfo[]> // list_backups
  openBackupDir(): Promise<void> // open_backup_dir — 폴더가 없으면 not_found
  hideMainWindow(): Promise<void> // hide_main_window — 플레이어 일시정지 후 트레이로 숨김(트레이 없으면 최소화)
  setTrayLabels(labels: TrayLabels): Promise<void> // set_tray_labels { labels } — 트레이 메뉴 문구를 표시 언어로 갱신(빈 문구는 무시)
  restoreBackup(input: RestoreBackupInput): Promise<void> // restore_backup { input } — 복원 전 자동 백업, 앱 재시작 (resolve 되지 않을 수 있다)
  takeStartupNotice(): Promise<StartupNotice | null> // take_startup_notice — 1회 읽기

  // P2-2 폴더·작업
  listFolders(): Promise<ManagedFolder[]> // list_folders
  addFolder(path: string, mediaTypes: MediaTypeSet): Promise<ManagedFolder> // add_folder { path, mediaTypes } — 모두 false 면 invalid_input
  relocateFolder(folderId: number, newPath: string): Promise<RelocateFolderResult> // relocate_folder { folderId, newPath } — 자산·태그 유지, 새 경로 스캔 예약
  removeFolder(folderId: number): Promise<void> // remove_folder { folderId } — 연결 데이터 삭제(백그라운드), 오류 backup_failed·busy
  rescanFolder(folderId: number): Promise<void> // rescan_folder { folderId }
  setFolderHidden(folderId: number, hidden: boolean): Promise<void> // set_folder_hidden { folderId, hidden }
  setFolderAlias(folderId: number, alias: string | null): Promise<void> // set_folder_alias { folderId, alias } — 공백 제거 후 빈 값은 null, 64자 초과는 invalid_input, 없는 폴더는 not_found
  setFolderMediaTypes(folderId: number, mediaTypes: MediaTypeSet, mode: MediaTypeChangeMode): Promise<SetFolderMediaTypesResult> // set_folder_media_types { folderId, mediaTypes, mode } — 모두 false 면 invalid_input, 없는 폴더는 not_found, keep 이면 detached=0
  countFolderAssetsByType(folderId: number): Promise<MediaTypeCounts> // count_folder_assets_by_type { folderId } — 활성 위치 기준, 없는 폴더는 not_found
  listSubfolders(folderId: number, relPath: string | null): Promise<FolderNode[]> // list_subfolders { folderId, relPath }
  jobSummary(): Promise<JobSummary> // job_summary
  pauseJobs(kind: JobKind | null): Promise<JobSummary> // pause_jobs { kind }
  resumeJobs(kind: JobKind | null): Promise<JobSummary> // resume_jobs { kind }
  listScanErrors(cursor: string | null, limit: number): Promise<Page<ScanError>> // list_scan_errors { cursor, limit }
  retryScanErrors(errorIds: number[] | null): Promise<number> // retry_scan_errors { errorIds } — null = 전체
  onJobsProgress(handler: (s: JobSummary) => void): Promise<() => void> // event jobs://progress

  // P2-5 검색
  searchAssets(req: SearchRequest): Promise<SearchPage> // search_assets { req }
  searchIndex(req: SearchIndexRequest): Promise<SearchIndex> // search_index { req }
  getAssetDetail(assetId: number): Promise<AssetDetail> // get_asset_detail { assetId }
  listRecentSearches(): Promise<RecentSearch[]> // list_recent_searches
  recordRecentSearch(query: string, filters: SearchFilters): Promise<void> // record_recent_search { query, filters }
  clearRecentSearches(): Promise<void> // clear_recent_searches

  // P2-7 사용자 데이터
  listTags(): Promise<Tag[]> // list_tags
  updateAssetTags(input: UpdateTagsInput): Promise<void> // update_asset_tags { input }
  renameTag(tagId: number, name: string): Promise<Tag> // rename_tag { tagId, name }
  deleteTag(tagId: number): Promise<void> // delete_tag { tagId }
  setMemo(assetId: number, memo: string): Promise<void> // set_memo { assetId, memo }
  setAssetTitle(assetId: number, title: string | null): Promise<void> // set_asset_title { assetId, title } — 공백 제거, 빈 값 = 해제, 최대 200자, 설정 꺼짐이면 invalid_input
  setFavorite(assetIds: number[], favorite: boolean): Promise<void> // set_favorite { assetIds, favorite }
  setRating(assetIds: number[], rating: number): Promise<void> // set_rating { assetIds, rating } — 0..5, 0 = 해제
  ratingCounts(): Promise<number[]> // rating_counts — 길이 6, index = 별점(0 = 미평가). 활성 + 파일 없음 제외 기준
  listCollections(): Promise<Collection[]> // list_collections
  createCollection(name: string): Promise<Collection> // create_collection { name }
  renameCollection(collectionId: number, name: string): Promise<Collection> // rename_collection { collectionId, name }
  deleteCollection(collectionId: number): Promise<void> // delete_collection { collectionId } — 자산은 보존
  addToCollection(collectionId: number, assetIds: number[]): Promise<void> // add_to_collection { collectionId, assetIds }
  removeFromCollection(collectionId: number, assetIds: number[]): Promise<void> // remove_from_collection { collectionId, assetIds }
  reorderCollection(collectionId: number, assetIds: number[], target: ReorderTarget): Promise<void> // reorder_collection { collectionId, assetIds, target } — 순서만 바꾼다(Outbox·revision 없음)
  collectionMembership(assetIds: number[]): Promise<CollectionMembership[]> // collection_membership { assetIds } — count 0 은 생략, 최대 1만 개
  collectionPlayQueue(collectionId: number): Promise<PlayQueueItem[]> // collection_play_queue { collectionId } — 컬렉션 순서 전체

  // P2-8 재활용
  pickDirectory(defaultPath?: string): Promise<string | null> // pick_directory { defaultPath? } — 생략하면 인자 없이 호출
  pickSaveFile(defaultFileName?: string, filters?: SaveFileFilter[]): Promise<string | null> // pick_save_file { defaultFileName?, filters? } — 경로만 돌려주고 파일은 쓰지 않는다. 취소 null. undefined 인자는 생략
  exportAssets(input: ExportInput): Promise<TaskStarted> // export_assets { input }
  onExportProgress(handler: (p: ExportProgress) => void): Promise<() => void> // event export://progress
  copyToClipboard(assetIds: number[]): Promise<void> // copy_to_clipboard { assetIds } — Windows 파일 클립보드(CF_HDROP)
  openOriginal(assetId: number): Promise<void> // open_original { assetId }
  revealInExplorer(assetId: number): Promise<void> // reveal_in_explorer { assetId }

  // P2-9 보호
  getProtectionStatus(): Promise<ProtectionStatus> // get_protection_status
  setPassword(password: string): Promise<ProtectionStatus> // set_password { password } — 최초 설정만
  changePassword(current: string, next: string): Promise<ProtectionStatus> // change_password { current, next }
  verifyPassword(password: string): Promise<ProtectionStatus> // verify_password { password } — 면제 시간 시작
  deleteOriginals(assetIds: number[]): Promise<OriginalsResult> // delete_originals { assetIds } — 휴지통으로, auth_required
  moveOriginals(assetIds: number[], destDir: string): Promise<OriginalsResult> // move_originals { assetIds, destDir } — 자산 연결 유지, auth_required
  resetManagedData(input: ResetInput): Promise<void> // reset_managed_data { input } — 백업 후 초기화, 앱 재시작

  // P1-7 자동 업데이트 — 설치는 사용자 승인 뒤에만 (PRD §2.7)
  checkUpdate(): Promise<UpdateCheck> // check_update
  installUpdate(): Promise<void> // install_update — 성공 시 앱이 종료되어 resolve 되지 않을 수 있다. reject 값은 UpdateError
  onUpdateState(handler: (p: UpdateStatePayload) => void): Promise<() => void> // event update-state
}

/** 썸네일 URL — 커스텀 프로토콜 `mamthumb://<asset_id>/<size>` (P2-3 D5). Windows WebView2 는 http://<scheme>.localhost 형식. */
export type ThumbSize = 320 | 1024

/** 원본 재생 URL — 커스텀 프로토콜 `mammedia://<asset_id>` (Range 지원 필요) */
export const MEDIA_SCHEME = "mammedia"
export const THUMB_SCHEME = "mamthumb"
