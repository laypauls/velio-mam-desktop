/**
 * 새 디스크 연결 확인 계약 — `docs/requests/2026-11/20261114-211201-new-volume-scan-prompt.md` "IPC".
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 */

export type PendingVolumeKind = "fixed" | "removable"

/** 앱 실행 중 새로 연결됐고 관리 폴더가 없는 볼륨 — 사용자 결정을 기다린다 */
export interface PendingVolume {
  volumeKey: string
  label: string | null
  kind: PendingVolumeKind
  /** 드라이브 루트 등 (예: `E:\`) */
  mountPoint: string
  totalBytes: number | null
}

/** "묻지 않기" 로 기억한 디스크 */
export interface IgnoredVolume {
  volumeKey: string
  label: string | null
  lastMountPoint: string | null
  /** UTC ms */
  ignoredAt: number
}

/** `volumes://pending-changed` 페이로드 — 대기 목록 전체 (변경 때마다) */
export interface PendingVolumesPayload {
  volumes: PendingVolume[]
}

export const PENDING_VOLUMES_EVENT = "volumes://pending-changed"

// ─── 관리 폴더 제거 · 기억하는 디스크 (`docs/requests/2026-11/20261114-220143-known-volume-cleanup.md` "IPC 계약") ───

/** 폴더 제거 시 영구히 지워지는 연결 데이터 개수 — 제거 확인 Dialog 에 표시 */
export interface FolderRemovalImpact {
  mediaCount: number
  tagLinks: number
  notes: number
  collectionItems: number
  aiResults: number
}

export type KnownVolumeKind = "fixed" | "removable" | "network"

/** 앱이 기억하는 디스크 — volumes 행 + 묻지 않음 항목을 volumeKey 로 병합한 1행 */
export interface KnownVolume {
  volumeKey: string
  label: string | null
  /** null = 묻지 않음 항목만 있고 volumes 행 없음 */
  kind: KnownVolumeKind | null
  lastMountPoint: string | null
  /** null = volumes 행 없음 */
  availability: "online" | "offline" | null
  /** UTC ms */
  lastSeenAt: number | null
  /** 활성 관리 폴더 수 (제거 진행 중 폴더 제외) */
  folderCount: number
  ignored: boolean
}

/** `folders://removal-progress` 페이로드 */
export interface FolderRemovalProgress {
  folderId: number
  done: number
  total: number
  finished: boolean
  error: string | null
}

export const FOLDER_REMOVAL_EVENT = "folders://removal-progress"

// ─── 디스크 꺼내기 준비 (`docs/requests/2026-11/20261114-231908-removable-safe-eject.md` "IPC 계약") ───

/** `prepare_volume_eject` 결과 — 같은 디스크의 관리 폴더 전부 */
export interface EjectPrepared {
  /** 드라이브 루트 등 (예: `E:\`) */
  mountPoint: string
  folderIds: number[]
}

export interface VolumeApi {
  /** prepare_volume_eject { folderId } — 네트워크 폴더는 `invalid_input`, 이미 오프라인이면 `offline`, 이미 준비 상태면 같은 결과로 성공 */
  prepareVolumeEject(folderId: number): Promise<EjectPrepared>
  /** resume_volume_watch { folderId } — 준비 상태가 아니면 아무 일 없이 성공 */
  resumeVolumeWatch(folderId: number): Promise<void>
  folderRemovalImpact(folderId: number): Promise<FolderRemovalImpact> // folder_removal_impact { folderId }
  listKnownVolumes(): Promise<KnownVolume[]> // list_known_volumes
  /** forget_volume_record { volumeKey } — 관리 폴더가 있으면 `volume_has_folders` */
  forgetVolumeRecord(volumeKey: string): Promise<void>
  onFolderRemovalProgress(handler: (p: FolderRemovalProgress) => void): Promise<() => void> // event folders://removal-progress
  listPendingNewVolumes(): Promise<PendingVolume[]> // list_pending_new_volumes
  dismissNewVolume(volumeKey: string, remember: boolean): Promise<void> // dismiss_new_volume { volumeKey, remember } — remember=true 면 ignored_volumes 에 저장
  listIgnoredVolumes(): Promise<IgnoredVolume[]> // list_ignored_volumes
  unignoreVolume(volumeKey: string): Promise<void> // unignore_volume { volumeKey } — 연결돼 있으면 바로 대기 목록으로
  onPendingVolumesChanged(handler: (volumes: PendingVolume[]) => void): Promise<() => void> // event volumes://pending-changed
}
