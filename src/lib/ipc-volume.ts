import { invoke } from "@tauri-apps/api/core"
import { listen, type UnlistenFn } from "@tauri-apps/api/event"
import {
  FOLDER_REMOVAL_EVENT,
  PENDING_VOLUMES_EVENT,
  type EjectPrepared,
  type FolderRemovalImpact,
  type FolderRemovalProgress,
  type IgnoredVolume,
  type KnownVolume,
  type PendingVolume,
  type PendingVolumesPayload,
} from "./ipc-contract-volume"

export { FOLDER_REMOVAL_EVENT, PENDING_VOLUMES_EVENT }

// 새 디스크 확인 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-volume.ts`.
// 등록은 기존 `addFolder(path, mediaTypes)` 를 재사용한다.

/** 앱 초기 로드 때 대기 목록 조회 */
export function listPendingNewVolumes(): Promise<PendingVolume[]> {
  return invoke<PendingVolume[]>("list_pending_new_volumes")
}

/** "나중에" — remember 면 이 디스크를 다시 묻지 않는다 */
export function dismissNewVolume(volumeKey: string, remember: boolean): Promise<void> {
  return invoke<void>("dismiss_new_volume", { volumeKey, remember })
}

export function listIgnoredVolumes(): Promise<IgnoredVolume[]> {
  return invoke<IgnoredVolume[]>("list_ignored_volumes")
}

/** 다시 묻기 — 지금 연결돼 있으면 바로 대기 목록에 오른다 */
export function unignoreVolume(volumeKey: string): Promise<void> {
  return invoke<void>("unignore_volume", { volumeKey })
}

/** 디스크 꺼내기 준비 — 같은 디스크의 감시·작업·사이드카·재생을 모두 멈춘 뒤 반환 */
export function prepareVolumeEject(folderId: number): Promise<EjectPrepared> {
  return invoke<EjectPrepared>("prepare_volume_eject", { folderId })
}

/** 꺼내기 준비 해제 + 재연결 처리 */
export function resumeVolumeWatch(folderId: number): Promise<void> {
  return invoke<void>("resume_volume_watch", { folderId })
}

/** 폴더 제거로 지워질 연결 데이터 개수 */
export function folderRemovalImpact(folderId: number): Promise<FolderRemovalImpact> {
  return invoke<FolderRemovalImpact>("folder_removal_impact", { folderId })
}

export function listKnownVolumes(): Promise<KnownVolume[]> {
  return invoke<KnownVolume[]>("list_known_volumes")
}

/** 디스크 기록 지우기 — 관리 폴더가 있으면 `volume_has_folders` 로 reject */
export function forgetVolumeRecord(volumeKey: string): Promise<void> {
  return invoke<void>("forget_volume_record", { volumeKey })
}

/** `folders://removal-progress` 구독. 반환 함수로 해제 */
export function onFolderRemovalProgress(handler: (p: FolderRemovalProgress) => void): Promise<UnlistenFn> {
  return listen<FolderRemovalProgress>(FOLDER_REMOVAL_EVENT, (e) => handler(e.payload))
}

/** `volumes://pending-changed` 구독 — 대기 목록 전체를 받는다. 반환 함수로 해제 */
export function onPendingVolumesChanged(handler: (volumes: PendingVolume[]) => void): Promise<UnlistenFn> {
  return listen<PendingVolumesPayload>(PENDING_VOLUMES_EVENT, (e) => handler(e.payload.volumes))
}
