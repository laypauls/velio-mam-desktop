/**
 * 표시 언어 파일 계약 — `docs/requests/2026-11/20261115-220746-i18n-locale-folder.md` "IPC 계약".
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 */

/** 언어 파일 검증 실패 사유 — 목록에는 남기되 선택할 수 없다 */
export type LocaleFileError = "invalid_json" | "invalid_shape" | "too_large" | "read_failed"

export interface LocaleInfo {
  /** 파일명에서 온 언어 코드 (BCP 47, 예: `en`, `zh-TW`). `ko` 는 내장 한국어를 덮어쓰는 파일 */
  code: string
  /** `_meta.name` (없으면 null → 코드로 표시) */
  name: string | null
  error: LocaleFileError | null
}

export interface LocaleList {
  /** 언어 폴더 절대 경로 (표시 전용) */
  dir: string
  /** 코드순 */
  locales: LocaleInfo[]
}

/** 언어 파일 내용 — 중첩 문자열 객체. `_meta` 가 있을 수 있다 */
export type LocaleResource = { [key: string]: string | LocaleResource }

export interface LocaleApi {
  listLocales(): Promise<LocaleList> // list_locales
  readLocale(code: string): Promise<LocaleResource> // read_locale { code } — 실패 invalid_input / not_found / io
  openLocaleFolder(): Promise<void> // open_locale_folder — 없으면 만든 뒤 탐색기로 연다
}
