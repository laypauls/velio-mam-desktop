import { invoke } from "@tauri-apps/api/core"
import type { ManagedFolder } from "./ipc"
import type { ChangeDetectorInfo, RescanPolicy, WatchOptions } from "./ipc-contract-watch"

// 변경 감지 방식 · 재스캔 정책 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-watch.ts`.

export function listChangeDetectors(folderId: number): Promise<ChangeDetectorInfo[]> {
  return invoke<ChangeDetectorInfo[]>("list_change_detectors", { folderId })
}

/** mode null = 자동 — 자동에서는 공통 옵션(`exclusiveOpenCheck`)만 받고, 감지기 전용 옵션은 invalid_input watchOptions */
export function setFolderWatchMode(folderId: number, mode: string | null, options: WatchOptions | null): Promise<ManagedFolder> {
  return invoke<ManagedFolder>("set_folder_watch_mode", { folderId, mode, options })
}

/** policy null = 전역 기본 */
export function setFolderRescanPolicy(folderId: number, policy: RescanPolicy | null): Promise<ManagedFolder> {
  return invoke<ManagedFolder>("set_folder_rescan_policy", { folderId, policy })
}
