import type { CommandError, ErrorCode } from "@/lib/ipc-contract"

// Record 로 두어 ErrorCode 에 코드가 추가되면 여기 빠진 것을 typecheck 가 잡는다 (누락 시 internal 문구로 떨어짐)
const KNOWN: Record<ErrorCode, true> = {
  invalid_input: true, not_found: true, already_registered: true, nested_folder: true, scan_running: true, offline: true,
  auth_required: true, password_incorrect: true, too_many_attempts: true, password_not_set: true,
  password_already_set: true, disk_full: true, io: true, internal: true,
  consent_required: true, provider_error: true, ai_busy: true, index_not_ready: true,
  backup_invalid: true, backup_too_new: true, backup_failed: true, volume_has_folders: true, busy: true,
  player_unavailable: true, asset_offline: true, open_failed: true, audio_device_unavailable: true,
  player_timeout: true, player_restarting: true, player_restart_failed: true, player_worker_crashed: true,
  plugin_invalid: true, plugin_incompatible: true, plugin_disabled: true, plugin_permission_denied: true, plugin_timeout: true, plugin_error: true,
}
const CODES: ReadonlySet<string> = new Set(Object.keys(KNOWN))

export function isCommandError(e: unknown): e is CommandError {
  return typeof e === "object" && e !== null && "code" in e && CODES.has((e as { code: string }).code)
}

/** 사용자 노출 메시지의 번역 키 (`errors.<code>`). 알 수 없으면 internal. */
export function errorKey(e: unknown): string {
  return `errors.${isCommandError(e) ? e.code : "internal"}`
}

/** 시작 직후(색인 재구축 중) 검색이 돌려주는 "색인 준비 중" 오류 — 전용 코드 `index_not_ready` (search_service.rs). */
export function isSearchNotReady(e: unknown): boolean {
  return isCommandError(e) && e.code === "index_not_ready"
}
