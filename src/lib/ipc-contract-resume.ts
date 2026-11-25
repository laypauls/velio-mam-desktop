/**
 * 이어보기(마지막 재생 위치) 계약 — `docs/requests/2026-11/20261122-111730-playback-resume.md` "IPC 계약 (확정)".
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 * 저장 판정(처음 5초·끝 5%/15초·최소 길이·사용 토글)은 Rust 가 한다 — 프론트는 위치를 그대로 보내고 결과만 받는다.
 */

/** 저장된 이어보기 위치 (원본 타임라인 정수 ms) */
export interface ResumePosition {
  positionMs: number
  durationMs: number | null
}

/** `resume_save` 결과 — saved: 기록함 / cleared: 처음·끝이라 기록을 지움 / skipped: 사용 안 함·짧은 콘텐츠 */
export type ResumeSaveOutcome = "saved" | "cleared" | "skipped"

/** 관리 폴더별 기록 수 (모든 관리 폴더 포함, 0건 포함). `total` 은 테이블 전체 행 수 — 어느 폴더에도 속하지 않는 기록 포함 */
export interface ResumeStats {
  folders: { folderId: number; count: number; enabled: boolean }[]
  total: number
}

/** 이어보기 최소 콘텐츠 길이 선택지(초) — 설정 키 `resumeMinDurationSec` 의 허용값 */
export const RESUME_MIN_DURATION_CHOICES = [0, 60, 180, 300, 600] as const

export interface ResumeApi {
  resumeGet(assetId: number): Promise<ResumePosition | null> // resume_get { assetId }
  resumeSave(assetId: number, positionMs: number, durationMs: number | null): Promise<ResumeSaveOutcome> // resume_save { assetId, positionMs, durationMs } — 없는 자산·음수 위치는 invalid_input/not_found
  resumeCountAssets(assetIds: number[]): Promise<number> // resume_count_assets { assetIds } — 기록 있는 자산 수
  resumeClearAssets(assetIds: number[]): Promise<number> // resume_clear_assets { assetIds } — 삭제 건수
  resumeClearFolder(folderId: number): Promise<number> // resume_clear_folder { folderId } — 삭제 건수
  resumeClearAll(): Promise<number> // resume_clear_all — 삭제 건수
  resumeStats(): Promise<ResumeStats> // resume_stats
  resumeSetFolderEnabled(folderId: number, enabled: boolean): Promise<void> // resume_set_folder_enabled { folderId, enabled } — 폴더별 하위 스위치(전역 resumeEnabled 가 켜져 있을 때만 적용, 기록은 유지). 없는 폴더는 not_found
}
