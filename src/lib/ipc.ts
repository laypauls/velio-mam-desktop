import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import type {
  AssetDetail,
  CollectionMembership,
  PlayQueueItem,
  ReorderTarget,
  CapturedAtFallback,
  DoubleClickAction,
  GalleryCardDisplay,
  GalleryPaging,
  IndexRailMode,
  LayoutSettings,
  LocaleList,
  LocaleResource,
  PlayerVolumeSettings,
  ScanStall,
  SourceHealth,
  StoredRescanPolicy,
  WatchOptions,
  PlayRange,
  PlayRangeKeys,
  RecentSearch,
  SearchFilters,
  SearchIndex,
  SearchIndexRequest,
  SearchPage,
  SceneSensitivity,
  SearchRequest,
  SaveFileFilter,
  SortFieldsEnabled,
  StartupNotice,
  SubtitleLanguage,
  ThemeSetting,
  TrayLabels,
  ViewerPlacement,
} from "./ipc-contract"

/** Rust `app_info` command 반환 형태 (src-tauri 와 동일 계약). */
export interface AppInfo {
  name: string
  version: string
}

export function getAppInfo(): Promise<AppInfo> {
  return invoke<AppInfo>("app_info")
}

/** Rust `check_update` 반환 (`update::UpdateCheck`, camelCase). 업데이트가 없으면 version/notes/date 는 null. */
export interface UpdateCheck {
  available: boolean
  currentVersion: string
  version: string | null
  notes: string | null
  date: string | null
}

/** Rust `UpdateError` 직렬화 형태 — command 가 reject 할 때의 값. */
export interface UpdateError {
  code:
    | "check_failed"
    | "not_checked"
    | "previous_installer_missing"
    | "prepare_failed"
    | "install_failed"
  message: string
}

/** `update-state` 이벤트 페이로드 (`update::UpdateStatePayload`). */
export interface UpdateStatePayload {
  phase: "downloading" | "installing" | "failed"
  downloaded: number
  total: number | null
}

export const UPDATE_STATE_EVENT = "update-state"

export function checkUpdate(): Promise<UpdateCheck> {
  return invoke<UpdateCheck>("check_update")
}

/**
 * 사용자가 승인한 뒤에만 호출한다 (요구사항 §2.7). 직전 `checkUpdate` 가 찾은 업데이트를 설치하며,
 * Windows 에서는 설치기가 시작되면 앱이 종료되므로 성공 시 resolve 되지 않을 수 있다.
 */
export function installUpdate(): Promise<void> {
  return invoke<void>("install_update")
}

export function onUpdateState(
  handler: (payload: UpdateStatePayload) => void,
): Promise<UnlistenFn> {
  return listen<UpdateStatePayload>(UPDATE_STATE_EVENT, (e) => handler(e.payload))
}

// ─── 저장소·설정 (P2-1) ──────────────────────────────────────────────────────
// Rust 쪽: src-tauri/src/{storage_commands,error}.rs, crates/mam-db/src/{settings,backup_set}.rs

/** 공용 command 오류 코드 (Rust `error::ErrorCode`, snake_case). */
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
  /** 외부 AI 전송 동의 전 요청 (P3) */
  | "consent_required"
  /** 외부 AI API 오류 — 메시지는 정제됨, 키 비포함 (P3) */
  | "provider_error"
  /** 진행 중 외부 AI 요청이 있어 구독 로그아웃을 거부 */
  | "ai_busy"
  | "disk_full"
  | "io"
  | "internal"
  /** 시작 직후 색인 재구축 중 — 잠시 뒤 재시도하면 된다 (P4 후속) */
  | "index_not_ready"
  /** 복원할 백업이 없거나 손상됨 */
  | "backup_invalid"
  /** 백업의 스키마 버전이 현재 앱보다 높음 */
  | "backup_too_new"
  /** 폴더 제거 직전 DB 백업 실패 — 아무것도 바꾸지 않았다 */
  | "backup_failed"
  /** 관리 폴더가 남은 디스크의 기록 지우기 거부 */
  | "volume_has_folders"
  /** 같은 범위의 폴더 제거가 진행 중 */
  | "busy"

/** P2 command 가 reject 할 때의 값. message 에는 내부 경로·키가 없다. */
export interface CommandError {
  code: ErrorCode
  message: string
}

/** `app_paths` — 절대 경로를 의도적으로 노출하는 유일한 command. */
export interface AppPaths {
  dataDir: string
  databasePath: string
  backupsDir: string
  logsDir: string
}

export interface BackupSettings {
  enabled: boolean
  /** 기본 24 (PL9) */
  intervalHours: number
  /** 기본 7 (PL9) */
  keep: number
  /** null = 기본 `MAMData/backups` */
  dir: string | null
}

/** `get_settings` / `update_settings` 반환. */
export interface Settings {
  /** 다음 실행부터 적용되는 관리 데이터 위치 (PL11). 로컬 드라이브만 허용 */
  dataDir: string
  /** 기본 6 */
  rescanIntervalHours: number
  /** 1~8, 기본 3 */
  maxConcurrentScans: number
  /** 비밀번호 인증 면제 시간 (PL1, 0~60, 기본 5) */
  authExemptMinutes: number
  backup: BackupSettings
  /** 기본 true */
  closeToTray: boolean
  /** 기본 false */
  firstRunCompleted: boolean
  /** 갤러리 카드 표시 옵션 — 프론트는 항상 전체 객체를 보낸다 (정의: ipc-contract.ts) */
  galleryCardDisplay: GalleryCardDisplay
  titleEditEnabled: boolean
  /** 원본 이동·삭제 동작 UI 노출 (기본 false) */
  originalActionsEnabled: boolean
  /** 기본 true */
  sidebarRatingMenu: boolean
  /** 기본 true */
  videoPlayerVlc: boolean
  /** 기본 "Escape", null = 사용 안 함 */
  hideShortcut: string | null
  /** 기본 "Q", null = 사용 안 함 */
  closeViewerShortcut: string | null
  /** 1~60, 기본 5 */
  slideshowImageSeconds: number
  sortFieldsEnabled: SortFieldsEnabled
  doubleClickAction: DoubleClickAction
  /** 기본 "modified" */
  capturedAtFallback: CapturedAtFallback
  /** 6~200, 기본 24 */
  similarMaxResults: number
  /** 0~0.99, 기본 0.75 (0 = 제한 없음) */
  similarMinSimilarity: number
  /** 기본 "infinite" */
  galleryPaging: GalleryPaging
  /** 50|100|200, 기본 100 */
  galleryPageSize: 50 | 100 | 200
  /** 기본 "auto" */
  indexRail: IndexRailMode
  /** 기본 { mode: "system", systemDark: "dim" } (정의: ipc-contract.ts) */
  theme: ThemeSetting
  /** 기본 "en" */
  language: string
  /** 기본 false (정지 상태로 열기) */
  mediaOpenAutoplay: boolean
  /** 기본 "brackets" */
  playRangeKeys: PlayRangeKeys
  /** 기본 false */
  playRangesOnly: boolean
  /** 기본 false */
  collectionPlayRangesOnly: boolean
  /** 기본 true */
  subtitleVisible: boolean
  /** 기본 "ko", null = 자막 끄고 시작 */
  subtitleDefaultLanguage: SubtitleLanguage | null
  /** 기본 true */
  sceneCutsVisible: boolean
  /** 기본 false */
  sceneDetectAuto: boolean
  /** 기본 "medium" */
  sceneDetectSensitivity: SceneSensitivity
  /** 기본 5 (정수 1~60) */
  playerPlayingStepSec: number
  /** 기본 "right" (갤러리 오른쪽), "left" = 창 맨 왼쪽 */
  viewerPlacement: ViewerPlacement
  /** 기본 { detailOpen: true, sidebarOpen: true, viewerWidth: 520, detailTab: "info", filmstripHeight: 104 } */
  layout: LayoutSettings
  /** 기본 { level: 100, muted: false } (level 은 0~100 정수). 음소거와 level 0 은 구분 저장 */
  playerVolume: PlayerVolumeSettings
  /** 기본 true (이어보기 사용) */
  resumeEnabled: boolean
  /** 기본 180 (허용 0·60·180·300·600) */
  resumeMinDurationSec: number
}

export type SettingsPatch = Partial<Omit<Settings, "backup" | "playerVolume">> & {
  backup?: Partial<BackupSettings>
  playerVolume?: Partial<PlayerVolumeSettings>
}

export type BackupKind = "auto" | "manual" | "pre_migrate" | "pre_reset" | "pre_restore" | "pre_remove"

/** `backup_now` / `list_backups`(최신 순) 항목. `createdAt` 은 UTC Unix ms. */
export interface BackupInfo {
  fileName: string
  createdAt: number
  sizeBytes: number
  kind: BackupKind
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings")
}

/** 범위·타입이 잘못된 값은 `invalid_input` 으로 reject 되고 아무것도 바뀌지 않는다. */
export function updateSettings(patch: SettingsPatch): Promise<Settings> {
  return invoke<Settings>("update_settings", { patch })
}

export function backupNow(): Promise<BackupInfo> {
  return invoke<BackupInfo>("backup_now")
}

export function listBackups(): Promise<BackupInfo[]> {
  return invoke<BackupInfo[]>("list_backups")
}

/** 백업 폴더를 탐색기로 연다. 폴더가 없으면 `not_found` (만들지 않는다). */
export function openBackupDir(): Promise<void> {
  return invoke<void>("open_backup_dir")
}

/** 언어 폴더의 언어 파일 목록 (코드순). 검증에 실패한 파일은 `error` 와 함께 목록에 남는다. */
export function listLocales(): Promise<LocaleList> {
  return invoke<LocaleList>("list_locales")
}

/** 언어 파일 내용(`_meta` 포함 가능). 실패는 `invalid_input` / `not_found` / `io`. */
export function readLocale(code: string): Promise<LocaleResource> {
  return invoke<LocaleResource>("read_locale", { code })
}

/** 언어 폴더를 탐색기로 연다 (없으면 만든다). */
export function openLocaleFolder(): Promise<void> {
  return invoke<void>("open_locale_folder")
}

/** 재생 중인 내장 플레이어를 일시정지하고 창을 트레이로 숨긴다(트레이가 없으면 최소화). */
export function hideMainWindow(): Promise<void> {
  return invoke<void>("hide_main_window")
}

/** 트레이 우클릭 메뉴 문구를 표시 언어로 갱신한다(번역·보간이 끝난 텍스트, 빈 문구는 코어가 무시). */
export function setTrayLabels(labels: TrayLabels): Promise<void> {
  return invoke<void>("set_tray_labels", { labels })
}

export function getAppPaths():Promise<AppPaths> {
  return invoke<AppPaths>("app_paths")
}

// ─── 수집: 관리 폴더·작업 (P2-2) ─────────────────────────────────────────────
// Rust 쪽: src-tauri/src/{ingest_commands,progress}.rs, crates/mam-core/src/{ingest,scan,jobs}
// 정본 계약은 UI 계약(`ipc-contract.ts`)이며 이름·모양이 같다.

/** 페이지 단위 조회 결과. `nextCursor` 가 null 이면 마지막 페이지. */
export interface Page<T> {
  items: T[]
  total: number
  nextCursor: string | null
}

/** 페이지 조회 `limit` 상한 (Rust 쪽도 이 값으로 자른다). */
export const MAX_PAGE_LIMIT = 200

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

/** `list_folders` / `add_folder` 항목 */
export interface ManagedFolder {
  folderId: number
  /** 표시용 절대 경로 (현재 드라이브 문자 기준) */
  path: string
  volumeKind: VolumeKind
  volumeLabel: string | null
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
  /** 관리 폴더 루트 기준 상대 경로, `/` 구분 */
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
  /** 전체 파일 수를 아직 모르면 null (사전 계수는 하지 않는다) */
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

export function listFolders(): Promise<ManagedFolder[]> {
  return invoke<ManagedFolder[]>("list_folders")
}

/** 중복은 `already_registered`, 상위/하위 폴더와 겹치면 `nested_folder`, 연결 안 된 공유는 `offline` 으로 reject. */
export function addFolder(path: string, mediaTypes: MediaTypeSet): Promise<ManagedFolder> {
  return invoke<ManagedFolder>("add_folder", { path, mediaTypes })
}

/**
 * 폴더를 새 경로로 옮겨 연결한다 (자산·태그·메모 유지, 새 경로 스캔 예약). 이전 경로는 읽지 않으므로 옛 볼륨이 오프라인이어도 된다.
 * 같은 경로·다른 관리 폴더와 같은 경로는 `already_registered`, 겹치면 `nested_folder`, 그 폴더 스캔이 실행 중이면 `scan_running`,
 * 새 경로가 없거나 폴더가 아니면 `invalid_input`, 연결 안 된 공유는 `offline` 으로 reject.
 */
export function relocateFolder(folderId: number, newPath: string): Promise<RelocateFolderResult> {
  return invoke<RelocateFolderResult>("relocate_folder", { folderId, newPath })
}

/**
 * 폴더를 제거 진행 상태로 바꾸고 반환한다 — 연결 데이터(메타정보·썸네일·태그·AI 결과) 삭제는 백그라운드로 이어진다
 * (`folders://removal-progress`). 사전 DB 백업 실패는 `backup_failed`, 같은 범위 진행 중은 `busy`. 원본 파일은 건드리지 않는다.
 */
export function removeFolder(folderId: number): Promise<void> {
  return invoke<void>("remove_folder", { folderId })
}

export function rescanFolder(folderId: number): Promise<void> {
  return invoke<void>("rescan_folder", { folderId })
}

/** 관리 폴더를 보기·검색에서 숨기거나 해제한다 (멱등). 없는 폴더·등록 해제 중이면 `not_found` 로 reject. */
export function setFolderHidden(folderId: number, hidden: boolean): Promise<void> {
  return invoke<void>("set_folder_hidden", { folderId, hidden })
}

/** 관리 폴더 별칭을 지정한다 (null 이면 제거). 64자 초과는 `invalid_input`, 없는 폴더는 `not_found` 로 reject. */
export function setFolderAlias(folderId: number, alias: string | null): Promise<void> {
  return invoke<void>("set_folder_alias", { folderId, alias })
}

/** 스캔 대상 유형을 바꾼다. 모두 false 는 `invalid_input`, 없는 폴더는 `not_found`. `purge` 는 제외된 유형의 연결 데이터를 삭제한다 (원본은 건드리지 않음). */
export function setFolderMediaTypes(folderId: number, mediaTypes: MediaTypeSet, mode: MediaTypeChangeMode): Promise<SetFolderMediaTypesResult> {
  return invoke<SetFolderMediaTypesResult>("set_folder_media_types", { folderId, mediaTypes, mode })
}

/** 폴더의 유형별 활성 자산 수. 없는 폴더는 `not_found` 로 reject. */
export function countFolderAssetsByType(folderId: number): Promise<MediaTypeCounts> {
  return invoke<MediaTypeCounts>("count_folder_assets_by_type", { folderId })
}

/** `relPath` null = 폴더 루트 바로 아래. 한 단계씩 지연 로딩한다. */
export function listSubfolders(folderId: number, relPath: string | null): Promise<FolderNode[]> {
  return invoke<FolderNode[]>("list_subfolders", { folderId, relPath })
}

export function jobSummary(): Promise<JobSummary> {
  return invoke<JobSummary>("job_summary")
}

/** `kind` null = 전역 일시정지 */
export function pauseJobs(kind: JobKind | null): Promise<JobSummary> {
  return invoke<JobSummary>("pause_jobs", { kind })
}

/** `kind` null = 전역·종류별 모두 재개 */
export function resumeJobs(kind: JobKind | null): Promise<JobSummary> {
  return invoke<JobSummary>("resume_jobs", { kind })
}

export function listScanErrors(cursor: string | null, limit: number): Promise<Page<ScanError>> {
  return invoke<Page<ScanError>>("list_scan_errors", { cursor, limit })
}

/** `errorIds` null = 전체. 재스캔을 예약한 오류 수를 돌려준다. */
export function retryScanErrors(errorIds: number[] | null): Promise<number> {
  return invoke<number>("retry_scan_errors", { errorIds })
}

/** `jobs://progress` 구독 (최대 4Hz). 반환 함수로 해제한다. */
export function onJobsProgress(handler: (s: JobSummary) => void): Promise<UnlistenFn> {
  return listen<JobSummary>(JOBS_PROGRESS_EVENT, (e) => handler(e.payload))
}

/** 폴더 선택 대화상자 (폴더 등록·내보내기 대상·백업 폴더용). 취소하면 null, 선택하면 절대 경로. */
export function pickDirectory(defaultPath?: string): Promise<string | null> {
  return defaultPath === undefined ? invoke<string | null>("pick_directory") : invoke<string | null>("pick_directory", { defaultPath })
}

/** 저장 위치 선택 대화상자 — 경로만 돌려주고 파일은 쓰지 않는다. 취소하면 null. undefined 인자는 invoke 에서 생략. */
export function pickSaveFile(defaultFileName?: string, filters?: SaveFileFilter[]): Promise<string | null> {
  const args: { defaultFileName?: string; filters?: SaveFileFilter[] } = {}
  if (defaultFileName !== undefined) args.defaultFileName = defaultFileName
  if (filters !== undefined) args.filters = filters
  return Object.keys(args).length === 0 ? invoke<string | null>("pick_save_file") : invoke<string | null>("pick_save_file", args)
}

// ─── 사용자 데이터 (P2-7) ────────────────────────────────────────────────────
// Rust 쪽: src-tauri/src/user_commands.rs, crates/mam-core/src/userdata. 정본 계약은 UI 계약(`ipc-contract.ts`)이며 이름·모양이 같다.
// 모든 변경은 revision 증가 + search_outbox(priority 0) 와 같은 트랜잭션이다.

export interface TagRef {
  tagId: number
  name: string
}

export interface Tag extends TagRef {
  assetCount: number
}

export interface CollectionRef {
  collectionId: number
  name: string
}

export interface Collection extends CollectionRef {
  assetCount: number
  createdAt: number
}

export interface UpdateTagsInput {
  assetIds: number[]
  /** 태그 이름 — 없으면 만든다 (이름 비교는 대소문자·NFC 정규화 기준) */
  add: string[]
  remove: number[]
}

/** 한 호출이 다루는 자산 수 상한 — 넘으면 `invalid_input`. 0개면 아무것도 하지 않는다. */
export const MAX_ASSETS_PER_CALL = 10_000

export function listTags(): Promise<Tag[]> {
  return invoke<Tag[]>("list_tags")
}

/** 없는 자산이 섞이면 `not_found` (아무것도 바뀌지 않는다). */
export function updateAssetTags(input: UpdateTagsInput): Promise<void> {
  return invoke<void>("update_asset_tags", { input })
}

/** 다른 태그와 이름이 같아지면 `already_registered`. */
export function renameTag(tagId: number, name: string): Promise<Tag> {
  return invoke<Tag>("rename_tag", { tagId, name })
}

export function deleteTag(tagId: number): Promise<void> {
  return invoke<void>("delete_tag", { tagId })
}

export function setMemo(assetId: number, memo: string): Promise<void> {
  return invoke<void>("set_memo", { assetId, memo })
}

/** 제목은 MAM 메타데이터일 뿐 원본 파일명을 바꾸지 않는다. 공백 제거·빈 값=해제·최대 200자, 설정 꺼짐이면 `invalid_input`. */
export function setAssetTitle(assetId: number, title: string | null): Promise<void> {
  return invoke<void>("set_asset_title", { assetId, title })
}

export function setFavorite(assetIds: number[], favorite: boolean): Promise<void> {
  return invoke<void>("set_favorite", { assetIds, favorite })
}

/** 0..5 (0 = 해제). 범위 밖 값은 `invalid_input`. */
export function setRating(assetIds: number[], rating: number): Promise<void> {
  return invoke<void>("set_rating", { assetIds, rating })
}

/** 길이 6 — index = 별점, 0 = 미평가. */
export function ratingCounts(): Promise<number[]> {
  return invoke<number[]>("rating_counts")
}

export function listCollections(): Promise<Collection[]> {
  return invoke<Collection[]>("list_collections")
}

/** 이름 중복(대소문자·NFC 정규화)은 `already_registered`. */
export function createCollection(name: string): Promise<Collection> {
  return invoke<Collection>("create_collection", { name })
}

export function renameCollection(collectionId: number, name: string): Promise<Collection> {
  return invoke<Collection>("rename_collection", { collectionId, name })
}

/** 컬렉션과 소속만 지운다 — 자산은 보존. */
export function deleteCollection(collectionId: number): Promise<void> {
  return invoke<void>("delete_collection", { collectionId })
}

export function addToCollection(collectionId: number, assetIds: number[]): Promise<void> {
  return invoke<void>("add_to_collection", { collectionId, assetIds })
}

export function removeFromCollection(collectionId: number, assetIds: number[]): Promise<void> {
  return invoke<void>("remove_from_collection", { collectionId, assetIds })
}

/** 선택 항목을 상대 순서 유지한 채 `target` 위치로 옮긴다 — 순서만 바뀌므로 색인 이벤트는 없다. */
export function reorderCollection(collectionId: number, assetIds: number[], target: ReorderTarget): Promise<void> {
  return invoke<void>("reorder_collection", { collectionId, assetIds, target })
}

/** 자산들이 속한 컬렉션별 개수 (count 0 은 생략). */
export function collectionMembership(assetIds: number[]): Promise<CollectionMembership[]> {
  return invoke<CollectionMembership[]>("collection_membership", { assetIds })
}

/** 연속 재생 대기열 — 컬렉션 순서 전체. */
export function collectionPlayQueue(collectionId: number): Promise<PlayQueueItem[]> {
  return invoke<PlayQueueItem[]>("collection_play_queue", { collectionId })
}

// ─── 검색·조회 (P2-5) ────────────────────────────────────────────────────────
// Rust 쪽: src-tauri/src/{search_commands,search_service}.rs, crates/mam-search/src/coordinator, crates/mam-core/src/query.
// 타입은 UI 계약(`ipc-contract.ts`)을 그대로 쓴다 (모양 중복 정의 없음).
// - `search_assets`: 필터는 모두 Tantivy 안에서 적용. 커서는 불투명 문자열이며 인덱스 재구축(세대 변경)·질의/필터/정렬 변경 시
//   `invalid_input` 이 되므로 UI 는 첫 페이지부터 다시 요청한다. `mode: "hybrid"` 는 벡터 후보가 준비되기 전까지
//   키워드 결과 + `degraded: true`. 인덱스를 아직 열지 못했으면(최초 구축 중) `internal`.
// - `direction: "backward"`(cursor 필수) 와 `at`(전역 위치 seek, cursor 와 동시 지정 불가) 도 같은 command. 모든 페이지가 `prevCursor`·`startOffset`·`generation` 을 싣는다.
//   `search_index` 는 같은 질의·필터·정렬의 구간(월·이름 첫 글자)별 개수 (목차 레일·행 모델용).
// - `limit` 은 1 ~ MAX_PAGE_LIMIT(200). `record_recent_search` 는 질의도 필터도 없으면 기록하지 않는다 (최대 50개 유지).

export type {
  AssetDetail,
  AssetSummary,
  FolderScope,
  RecentSearch,
  SearchFilters,
  SearchIndex,
  SearchIndexRequest,
  SearchPage,
  SearchRequest,
  SortSpec,
} from "./ipc-contract"

export function searchAssets(req: SearchRequest): Promise<SearchPage> {
  return invoke<SearchPage>("search_assets", { req })
}

export function searchIndex(req: SearchIndexRequest): Promise<SearchIndex> {
  return invoke<SearchIndex>("search_index", { req })
}

export function getAssetDetail(assetId: number): Promise<AssetDetail> {
  return invoke<AssetDetail>("get_asset_detail", { assetId })
}

export function listRecentSearches(): Promise<RecentSearch[]> {
  return invoke<RecentSearch[]>("list_recent_searches")
}

export function recordRecentSearch(query: string, filters: SearchFilters): Promise<void> {
  return invoke<void>("record_recent_search", { query, filters })
}

export function clearRecentSearches(): Promise<void> {
  return invoke<void>("clear_recent_searches")
}

// ─── 재활용 (P2-8) ───────────────────────────────────────────────────────────
// Rust 쪽: src-tauri/src/reuse_commands.rs, crates/mam-core/src/reuse

export type ConflictPolicy = "skip" | "rename" | "overwrite"

export interface ExportInput {
  assetIds: number[]
  /** 존재하는 폴더. 관리 폴더·MAM 데이터 안이면 `invalid_input` */
  destDir: string
  conflict: ConflictPolicy
  /** 대상 폴더에 메타데이터 CSV(mam-export.csv)를 함께 쓴다 */
  includeManifest: boolean
}

export interface TaskStarted {
  taskId: string
}

/** `export://progress` 페이로드. `done` = 처리한 건수(성공 + 실패 + 건너뜀). 마지막 이벤트는 `finished: true`. */
export interface ExportProgress {
  taskId: string
  done: number
  total: number
  failed: number
  skipped: number
  finished: boolean
}

export const EXPORT_PROGRESS_EVENT = "export://progress"

/** 시작만 하고 `taskId` 를 돌려준다. 진행은 `onExportProgress` — 이벤트가 응답보다 먼저 올 수 있다. */
export function exportAssets(input: ExportInput): Promise<TaskStarted> {
  return invoke<TaskStarted>("export_assets", { input })
}

/** `export://progress` 구독 (최대 4Hz + 마지막 1회). 반환 함수로 해제한다. */
export function onExportProgress(handler: (p: ExportProgress) => void): Promise<UnlistenFn> {
  return listen<ExportProgress>(EXPORT_PROGRESS_EVENT, (e) => handler(e.payload))
}

/** Windows 파일 클립보드(CF_HDROP)에 원본 경로를 넣는다. 오프라인·파일 없음 자산은 제외, 하나도 없으면 `offline`/`not_found`. */
export function copyToClipboard(assetIds: number[]): Promise<void> {
  return invoke<void>("copy_to_clipboard", { assetIds })
}

/** 기본 연결 프로그램으로 연다. 오프라인이면 `offline`, 파일이 없으면 `not_found`. */
export function openOriginal(assetId: number): Promise<void> {
  return invoke<void>("open_original", { assetId })
}

/** 탐색기에서 파일을 선택한 상태로 연다. 오프라인이면 `offline`, 파일이 없으면 `not_found`. */
export function revealInExplorer(assetId: number): Promise<void> {
  return invoke<void>("reveal_in_explorer", { assetId })
}

// ─── 보호·운영 (P2-9) ────────────────────────────────────────────────────────
// Rust 쪽: src-tauri/src/{protect_commands,protection,reset,tray,backup_scheduler}.rs, crates/mam-core/src/protect
// 비밀번호는 로그·상태에 남기지 않는다. 평문은 호출 인자로만 전달한다.

/** `get_protection_status` 등 반환. `authValidUntil` 은 UTC ms, null = 인증 안 됨 (면제 0분이면 항상 null). */
export interface ProtectionStatus {
  passwordSet: boolean
  authValidUntil: number | null
}

/** 삭제·이동 결과 — 자산별 실패가 있어도 나머지는 처리된다. */
export interface OriginalsResult {
  succeeded: number
  failed: { assetId: number; message: string }[]
}

export interface ResetInput {
  /** 비밀번호를 아는 경우 */
  password: string | null
  /** 비밀번호 분실 경로 — 인증 없이 관리 데이터 초기화 (원본 무관, PL3) */
  forgotPassword: boolean
}

export function getProtectionStatus(): Promise<ProtectionStatus> {
  return invoke<ProtectionStatus>("get_protection_status")
}

/** 최초 설정만. 8자 미만은 `invalid_input`, 이미 있으면 `password_already_set`. 인증 면제는 시작하지 않는다. */
export function setPassword(password: string): Promise<ProtectionStatus> {
  return invoke<ProtectionStatus>("set_password", { password })
}

/** 현재 비밀번호가 틀리면 `password_incorrect`, 미설정이면 `password_not_set`. */
export function changePassword(current: string, next: string): Promise<ProtectionStatus> {
  return invoke<ProtectionStatus>("change_password", { current, next })
}

/** 성공하면 인증 면제 시간이 시작된다. 연속 5회 실패하면 30초간 `too_many_attempts`. */
export function verifyPassword(password: string): Promise<ProtectionStatus> {
  return invoke<ProtectionStatus>("verify_password", { password })
}

/** Windows 휴지통으로 이동한다(영구 삭제 없음). 인증 면제 밖이면 `auth_required`. */
export function deleteOriginals(assetIds: number[]): Promise<OriginalsResult> {
  return invoke<OriginalsResult>("delete_originals", { assetIds })
}

/** 대상 폴더로 이동하고 자산 연결을 유지한다. 인증 면제 밖이면 `auth_required`. */
export function moveOriginals(assetIds: number[], destDir: string): Promise<OriginalsResult> {
  return invoke<OriginalsResult>("move_originals", { assetIds, destDir })
}

/**
 * 백업 후 관리 데이터(DB·인덱스·캐시·설정)를 초기화하고 앱을 다시 시작한다 — 원본·백업 폴더는 건드리지 않는다.
 * 성공하면 앱이 재시작되므로 resolve 되지 않을 수 있다. 백업 실패는 `io`, 비밀번호 오류는 `password_incorrect`.
 */
export function resetManagedData(input: ResetInput): Promise<void> {
  return invoke<void>("reset_managed_data", { input })
}

/**
 * 백업으로 복원한다 — 복원 전 현재 DB 를 `pre_restore` 로 백업하고 앱을 다시 시작한다(resolve 되지 않을 수 있다).
 * 오류: `auth_required`(비밀번호 설정 시 미입력)·`password_incorrect`·`backup_invalid`·`backup_too_new`·`not_found`.
 */
export function restoreBackup(input: { fileName: string; password?: string }): Promise<void> {
  return invoke<void>("restore_backup", { input })
}

/** 기동 시 복원/자동 복구 알림을 1회 읽는다(읽으면 지워진다). 없으면 null. */
export function takeStartupNotice(): Promise<StartupNotice | null> {
  return invoke<StartupNotice | null>("take_startup_notice")
}

// ─── P3 AI (ipc-contract-ai.ts) ──────────────────────────────────────────────
// Rust 쪽: src-tauri/src/{ai_commands.rs,ai/}. 키(API)는 `set_api_key` 인자로만 전달하고 어떤 응답에도 되돌아오지 않는다.
// 오류: 동의 전 외부 분석 `consent_required`, 범위·형식 오류 `invalid_input`, 없는 자산 `not_found`.
// `check_provider` 는 실패해도 reject 하지 않고 `{ ok:false, errorCode }` 로 돌려준다 (auth_failed | offline | rate_limited | unknown).
// `download_model` 은 시작만 하고 바로 resolve 한다 — 진행·완료·실패는 `models://progress`.
// `ai://analysis` 이벤트: 자산별 분석 상태 변화 (UI 폴링은 대량 등록 경로의 보조로 유지한다).

export {
  ANALYSIS_EVENT,
  MODELS_PROGRESS_EVENT,
  type AiAnalysis,
  type AiCliLoginState,
  type AiProvider,
  type AiSettings,
  type AiSettingsPatch,
  type AiUsageRow,
  type AnalysisChange,
  type AnalysisEdit,
  type AnalysisEvent,
  type AnalysisKind,
  type AnalysisRun,
  type AssetAiDetail,
  type AuthMode,
  type CliAccount,
  type CliStatus,
  type LocalModel,
  type ModelProgress,
  type ProviderCheck,
  type ProviderStatus,
  type QueueResult,
  type SimilarItem,
  type SimilarRequest,
} from "./ipc-contract-ai"
import type {
  AiAnalysis,
  AiCliLoginState,
  AiProvider,
  AiSettings,
  AiSettingsPatch,
  AiUsageRow,
  AnalysisChange,
  AnalysisEdit,
  AnalysisEvent,
  AnalysisKind,
  AnalysisRun,
  AssetAiDetail,
  AuthMode,
  LocalModel,
  ModelProgress,
  ProviderCheck,
  ProviderStatus,
  QueueResult,
} from "./ipc-contract-ai"
import { ANALYSIS_EVENT, MODELS_PROGRESS_EVENT } from "./ipc-contract-ai"

export function getAiSettings(): Promise<AiSettings> {
  return invoke<AiSettings>("get_ai_settings")
}

export function updateAiSettings(patch: AiSettingsPatch): Promise<AiSettings> {
  return invoke<AiSettings>("update_ai_settings", { patch })
}

export function giveAiConsent(): Promise<AiSettings> {
  return invoke<AiSettings>("give_ai_consent")
}

export function revokeAiConsent(): Promise<AiSettings> {
  return invoke<AiSettings>("revoke_ai_consent")
}

export function listAiProviders(): Promise<ProviderStatus[]> {
  return invoke<ProviderStatus[]>("list_ai_providers")
}

export function setApiKey(provider: AiProvider, key: string, persist: boolean): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("set_api_key", { provider, key, persist })
}

export function clearApiKey(provider: AiProvider): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("clear_api_key", { provider })
}

export function setProviderModel(provider: AiProvider, model: string | null): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("set_provider_model", { provider, model })
}

export function checkProvider(provider: AiProvider): Promise<ProviderCheck> {
  return invoke<ProviderCheck>("check_provider", { provider })
}

export function setProviderAuthMode(provider: AiProvider, mode: AuthMode): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("set_provider_auth_mode", { provider, mode })
}

export function detectAiCli(provider: AiProvider): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("detect_ai_cli", { provider })
}

export function aiCliLoginStart(provider: AiProvider): Promise<AiCliLoginState> {
  return invoke<AiCliLoginState>("ai_cli_login_start", { provider })
}

export function aiCliLoginStatus(provider: AiProvider): Promise<AiCliLoginState> {
  return invoke<AiCliLoginState>("ai_cli_login_status", { provider })
}

export function aiCliLoginCancel(provider: AiProvider): Promise<AiCliLoginState> {
  return invoke<AiCliLoginState>("ai_cli_login_cancel", { provider })
}

/** 인자는 공급자뿐 — 열 URL 은 Rust 가 CLI 출력에서 캡처한 것만 쓴다. */
export function openAiCliLoginUrl(provider: AiProvider): Promise<boolean> {
  return invoke<boolean>("open_ai_cli_login_url", { provider })
}

export function aiCliLogout(provider: AiProvider): Promise<ProviderStatus> {
  return invoke<ProviderStatus>("ai_cli_logout", { provider })
}

export function listLocalModels(): Promise<LocalModel[]> {
  return invoke<LocalModel[]>("list_local_models")
}

export function downloadModel(modelId: string): Promise<void> {
  return invoke<void>("download_model", { modelId })
}

export function onModelsProgress(handler: (p: ModelProgress) => void): Promise<UnlistenFn> {
  return listen<ModelProgress>(MODELS_PROGRESS_EVENT, (e) => handler(e.payload))
}

/** `ai://analysis` 구독 — 상태가 바뀐 자산들을 받는다. */
export function onAnalysisChanged(handler: (changes: AnalysisChange[]) => void): Promise<UnlistenFn> {
  return listen<AnalysisEvent>(ANALYSIS_EVENT, (e) => handler(e.payload.changes))
}

export function analyzeAssets(assetIds: number[], kinds: AnalysisKind[] | null, force: boolean): Promise<QueueResult> {
  return invoke<QueueResult>("analyze_assets", { assetIds, kinds, force })
}

export function analyzeAll(onlyUnanalyzed: boolean): Promise<QueueResult> {
  return invoke<QueueResult>("analyze_all", { onlyUnanalyzed })
}

export function retryFailedAnalysis(assetIds: number[] | null): Promise<QueueResult> {
  return invoke<QueueResult>("retry_failed_analysis", { assetIds })
}

export function getAssetAi(assetId: number): Promise<AssetAiDetail> {
  return invoke<AssetAiDetail>("get_asset_ai", { assetId })
}

export function updateAnalysis(assetId: number, edit: AnalysisEdit): Promise<AiAnalysis> {
  return invoke<AiAnalysis>("update_analysis", { assetId, edit })
}

export function listAnalysisHistory(assetId: number): Promise<AnalysisRun[]> {
  return invoke<AnalysisRun[]>("list_analysis_history", { assetId })
}

export function listAiUsage(fromDay: string, toDay: string): Promise<AiUsageRow[]> {
  return invoke<AiUsageRow[]>("list_ai_usage", { fromDay, toDay })
}

// ─── 내장 플레이어 (libmpv 워커) ─────────────────────────────────────────────
// Rust 쪽: src-tauri 의 player command, crates/mam-player. 정본 계약은 `ipc-contract-player.ts`, 프레임 와이어는 `ipc-player-wire.ts`.
// `player_open` 오류: `player_unavailable` / `asset_offline` / `open_failed` / `player_timeout`. 크기는 캔버스 장치 px.

import { PLAYER_STATE_EVENT } from "./ipc-contract-player"

export * from "./ipc-resume" // resumeGet, resumeSave, resumeCountAssets, resumeClearAssets, resumeClearFolder, resumeClearAll, resumeStats, resumeSetFolderEnabled
export * from "./ipc-ranges" // playRanges*, collectionPlayRangeSummary, playerSetRanges
export * from "./ipc-subtitle" // subtitleTracks, subtitleCues, playerSetSubtitle
export * from "./ipc-volume" // listPendingNewVolumes, dismissNewVolume, listIgnoredVolumes, unignoreVolume, onPendingVolumesChanged, folderRemovalImpact, listKnownVolumes, forgetVolumeRecord, onFolderRemovalProgress
export * from "./ipc-watch" // listChangeDetectors, setFolderWatchMode, setFolderRescanPolicy
export * from "./ipc-remote" // listRemoteSources, createRemoteSource, updateRemoteSource, deleteRemoteSource, testRemoteSource
export * from "./ipc-scene" // sceneCuts*, sceneCut*, sceneDetect*, sceneBackfillCount, onSceneUpdated
export * from "./ipc-plugin" // plugin*, onPluginChanged, PLUGIN_CHANGED_EVENT, PLUGIN_PERMISSIONS (+ 계약 타입)
export * from "./ipc-metadata" // listMetadataFields, createMetadataField, ..., getAssetMetadata, setAssetMetadata, bulkSetAssetMetadata
import type { CanvasSize, PlayerControlAction, PlayerOpenResult, PlayerStatePayload } from "./ipc-contract-player"
import type { ReleaseMsg } from "./ipc-player-wire"

export { PLAYER_STATE_EVENT }
export type { CanvasSize, PlayerControlAction, PlayerOpenResult, PlayerStatePayload }

export function playerAvailable(): Promise<boolean> {
  return invoke<boolean>("player_available")
}

/**
 * `autoplay` 는 필수 — false 면 첫 프레임에서 일시정지한 채 열린다. 첫 프레임이 게시된 뒤에 응답한다 (최대 20초, 초과 시 `player_timeout`).
 * `canvas` = 영상 영역의 장치 픽셀 크기. `ranges`(정수 ms, 선택)를 주면 구간만 재생이 열기와 함께 켜져 첫 구간 안에서 시작한다. 생략/null = 전체 재생 — 키 자체를 보내지 않는다(undefined).
 * `startMs`(정수 ms, 선택) = 이어보기 시작 위치 — null/생략이면 키를 보내지 않는다(기존 페이로드 그대로). 구간만 재생이면 구간 안으로 보정한 값을 넘긴다.
 */
export function playerOpen(assetId: number, canvas: CanvasSize, autoplay: boolean, ranges?: PlayRange[] | null, startMs?: number | null): Promise<PlayerOpenResult> {
  const args = {
    assetId,
    autoplay,
    canvas,
    ...(ranges === undefined ? {} : { ranges }),
    ...(startMs === undefined || startMs === null ? {} : { startMs }),
  }
  return invoke<PlayerOpenResult>("player_open", args)
}

/** `stepBack` = 일시정지 후 한 프레임 뒤로. 이동 뒤의 시각은 `player://state` 이벤트가 알려 준다 (`fps` 인자 없음) */
export function playerControl(action: PlayerControlAction): Promise<void> {
  return invoke<void>("player_control", { action })
}

export function playerSeek(ms: number): Promise<void> {
  return invoke<void>("player_seek", { ms })
}

export function playerSetRate(rate: number): Promise<void> {
  return invoke<void>("player_set_rate", { rate })
}

/** `volume` 0..100 */
export function playerSetVolume(volume: number, muted: boolean): Promise<void> {
  return invoke<void>("player_set_volume", { volume, muted })
}

/** Windows 마스터 음량 상대 변경 — 반환 = 변경 후 0..1. 장치가 없으면 `audio_device_unavailable` 로 reject */
export function systemVolumeStep(delta: number): Promise<number> {
  return invoke<number>("system_volume_step", { delta })
}

/** 캔버스(장치 px)가 바뀌었다 — 호스트가 새 에포크의 버퍼를 만든다. 호출부가 디바운스한다 */
export function playerSetRenderSize(width: number, height: number): Promise<void> {
  return invoke<void>("player_set_render_size", { width, height })
}

/** false 면 워커가 프레임을 만들지 않는다 (창 숨김·보기 영역 접힘) */
export function playerSetRenderActive(active: boolean): Promise<void> {
  return invoke<void>("player_set_render_active", { active })
}

/** 그린 프레임 슬롯을 호스트에 돌려준다 — 프레임마다 fire-and-forget (`chrome.webview.postMessage` 는 Tauri 가 객체 메시지를 거부해 호스트에 닿지 않는다) */
export function playerReleaseSlots(release: ReleaseMsg): Promise<void> {
  return invoke<void>("player_release_slots", { release })
}

/** 워커 재시작 실패(`restartFailed`) 뒤 다시 시도 — 열기와 같은 응답 */
export function playerRestart(): Promise<PlayerOpenResult> {
  return invoke<PlayerOpenResult>("player_restart")
}

export function playerClose(): Promise<void> {
  return invoke<void>("player_close")
}

/** `player://state` 구독 (재생 중 시간 갱신 최대 5Hz). 반환 함수로 해제한다. */
export function onPlayerState(handler: (p: PlayerStatePayload) => void): Promise<UnlistenFn> {
  return listen<PlayerStatePayload>(PLAYER_STATE_EVENT, (e) => handler(e.payload))
}
