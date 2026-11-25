/**
 * 사용자 정의 메타데이터 필드 계약 — `docs/requests/2026-11/20261115-135644-custom-metadata-fields.md` D1·D3.
 * 규칙은 `ipc-contract.ts` 와 같다 (snake_case command, camelCase serde, 실패 시 CommandError). `ipc-contract.ts` 가 다시 내보낸다.
 * Rust 쪽: `src-tauri/src/metadata_commands.rs`·`metadata_types.rs`, 필터는 `mam_core::query::UserFieldFilter`.
 *
 * 오류 코드: `invalid_input`(이름·값 형식·타입 불일치·필드 20개 초과·자산 10,000건 초과·삭제 중이거나 없는 필드/선택지를 가리키는 필터),
 * `not_found`(없거나 삭제 중인 필드·선택지·자산), `already_registered`(같은 이름의 필드, 같은 필드 안 같은 이름의 선택지).
 * 필드 이름·선택지 이름은 사용자 데이터라 번역 키 대상이 아니다.
 */

/** 필드 값 타입 — 생성 후 변경 불가. 화면 이름: 짧은 글 / 숫자 / 날짜 / 예·아니요 / 선택 / 다중 선택 */
export type MetadataValueType = "keyword" | "number" | "date" | "boolean" | "enum" | "multiEnum"

/** 필드 최대 개수 (Rust `MAX_METADATA_FIELDS`) */
export const MAX_METADATA_FIELDS = 20
/** keyword 값 최대 글자 수 (trim + NFC 후, Rust `MAX_KEYWORD_CHARS`) */
export const MAX_KEYWORD_CHARS = 200
/** 대량 지정 한 번에 다룰 수 있는 자산 수 (초과 시 `invalid_input`) */
export const MAX_BULK_ASSETS = 10_000

export interface MetadataFieldOption {
  optionId: number
  label: string
  position: number
}

export interface MetadataField {
  fieldId: number
  name: string
  valueType: MetadataValueType
  position: number
  /** enum / multiEnum 만 값이 있다 (표시 순서). 그 외 타입은 빈 배열 */
  options: MetadataFieldOption[]
  createdAt: number
  updatedAt: number
}

/**
 * 필드 값 — 타입을 `kind` 로 함께 싣는다 (date 와 enum 이 모두 정수라 값만으로는 구분되지 않는다).
 * date 는 `YYYYMMDD` 정수(유효한 달력 날짜), number 는 유한한 f64, enum 은 optionId, multiEnum 은 optionId 배열,
 * keyword 는 trim + NFC 후 200자 이하(빈 문자열 = 값 없음).
 */
export type MetadataValue =
  | { kind: "keyword"; value: string }
  | { kind: "number"; value: number }
  | { kind: "date"; value: number }
  | { kind: "boolean"; value: boolean }
  | { kind: "enum"; value: number }
  | { kind: "multiEnum"; value: number[] }

/** `get_asset_metadata` 항목 — 값이 있는 필드만 필드 표시 순서로 온다 */
export interface AssetMetadataValue {
  fieldId: number
  value: MetadataValue
}

/** 필드 하나에 대한 변경 — 패치에 없는 필드는 그대로 둔다 */
export type MetadataPatch =
  | { op: "set"; fieldId: number; value: MetadataValue }
  | { op: "clear"; fieldId: number }

/**
 * 갤러리 필터의 사용자 필드 조건 — `SearchFilters.userFields`. 조건끼리는 AND.
 * 조건 종류는 필드 타입과 맞아야 한다 (`has`/`missing` 은 모든 타입). 한 검색에 최대 40개.
 * - keywordAnyOf: 여러 값 중 하나 (대소문자·NFC 무시, 비어 있지 않아야 함). 일치 = 값 1개
 * - numberRange / dateRange: 닫힌 구간, 한쪽만 주면 그쪽은 열림 (둘 다 null 이면 `has` 를 쓸 것). 일치 = [x, x]
 * - bool, enumAnyOf(비어 있지 않은 optionId), multiEnum(any / `all: true` 면 모두 포함)
 */
export type UserFieldFilter =
  | { kind: "keywordAnyOf"; fieldId: number; values: string[] }
  | { kind: "numberRange"; fieldId: number; min: number | null; max: number | null }
  | { kind: "dateRange"; fieldId: number; min: number | null; max: number | null }
  | { kind: "bool"; fieldId: number; value: boolean }
  | { kind: "enumAnyOf"; fieldId: number; optionIds: number[] }
  | { kind: "multiEnum"; fieldId: number; optionIds: number[]; all: boolean }
  | { kind: "has"; fieldId: number }
  | { kind: "missing"; fieldId: number }
