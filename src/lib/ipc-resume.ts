import { invoke } from "@tauri-apps/api/core"
import type { ResumePosition, ResumeSaveOutcome, ResumeStats } from "./ipc-contract-resume"

// 이어보기 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-resume.ts`.

/** 자산의 저장된 이어보기 위치 (없으면 null) */
export function resumeGet(assetId: number): Promise<ResumePosition | null> {
  return invoke<ResumePosition | null>("resume_get", { assetId })
}

/** 현재 위치 저장 — 위치는 정수 ms 여야 한다(소수는 command 진입 전 거부). 길이를 모르면 null */
export function resumeSave(assetId: number, positionMs: number, durationMs: number | null): Promise<ResumeSaveOutcome> {
  return invoke<ResumeSaveOutcome>("resume_save", { assetId, positionMs, durationMs })
}

/** 선택 자산 중 이어보기 기록이 있는 수 */
export function resumeCountAssets(assetIds: number[]): Promise<number> {
  return invoke<number>("resume_count_assets", { assetIds })
}

/** 선택 자산의 이어보기 기록 삭제 — 삭제 건수 */
export function resumeClearAssets(assetIds: number[]): Promise<number> {
  return invoke<number>("resume_clear_assets", { assetIds })
}

/** 관리 폴더 안 자산의 이어보기 기록 삭제 (활성 위치 기준) — 삭제 건수 */
export function resumeClearFolder(folderId: number): Promise<number> {
  return invoke<number>("resume_clear_folder", { folderId })
}

/** 모든 이어보기 기록 삭제 — 삭제 건수 */
export function resumeClearAll(): Promise<number> {
  return invoke<number>("resume_clear_all")
}

/** 관리 폴더별 이어보기 사용 여부 변경 — 기록은 지우지 않는다. 없는 폴더는 not_found */
export function resumeSetFolderEnabled(folderId: number, enabled: boolean): Promise<void> {
  return invoke<void>("resume_set_folder_enabled", { folderId, enabled })
}

/** 관리 폴더별·전체 기록 수 */
export function resumeStats(): Promise<ResumeStats> {
  return invoke<ResumeStats>("resume_stats")
}
