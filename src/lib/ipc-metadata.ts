import { invoke } from "@tauri-apps/api/core"
import type {
  AssetMetadataValue,
  MetadataField,
  MetadataFieldOption,
  MetadataPatch,
  MetadataValueType,
} from "./ipc-contract-metadata"

// 사용자 정의 메타데이터 필드 래퍼 — `ipc.ts` 가 다시 내보낸다. 정본 계약: `ipc-contract-metadata.ts`.
// 값을 바꾸는 command 는 색인 워커를 깨우고, 삭제 command 는 "삭제 중" 표시 뒤 바로 돌아온다 (값 정리는 백그라운드).

/** 필드 목록 (표시 순서, 선택지 포함, 삭제 중인 필드 제외) */
export function listMetadataFields(): Promise<MetadataField[]> {
  return invoke<MetadataField[]>("list_metadata_fields")
}

/** 필드 생성 — `options` 는 enum/multiEnum 에서만 허용(그 외 타입은 []). 같은 이름 → already_registered, 21번째 → invalid_input */
export function createMetadataField(name: string, valueType: MetadataValueType, options: string[]): Promise<MetadataField> {
  return invoke<MetadataField>("create_metadata_field", { name, valueType, options })
}

/** 이름만 변경 (재색인 없음) */
export function renameMetadataField(fieldId: number, name: string): Promise<MetadataField> {
  return invoke<MetadataField>("rename_metadata_field", { fieldId, name })
}

/** 현재 필드 전체의 새 순서 — 누락·중복·모르는 id 는 invalid_input */
export function reorderMetadataFields(fieldIds: number[]): Promise<void> {
  return invoke<void>("reorder_metadata_fields", { fieldIds })
}

/** 필드 삭제 요청 — 값을 가진 자산 수를 돌려준다. 즉시 목록·필터에서 사라지고 값 정리는 백그라운드. 복원 불가 */
export function deleteMetadataField(fieldId: number): Promise<number> {
  return invoke<number>("delete_metadata_field", { fieldId })
}

/** 이 필드에 값을 가진 자산 수 (삭제 확인창) */
export function metadataFieldValueCount(fieldId: number): Promise<number> {
  return invoke<number>("metadata_field_value_count", { fieldId })
}

export function addMetadataOption(fieldId: number, label: string): Promise<MetadataFieldOption> {
  return invoke<MetadataFieldOption>("add_metadata_option", { fieldId, label })
}

/** 이름만 변경 (재색인 없음) */
export function renameMetadataOption(optionId: number, label: string): Promise<MetadataFieldOption> {
  return invoke<MetadataFieldOption>("rename_metadata_option", { optionId, label })
}

export function reorderMetadataOptions(fieldId: number, optionIds: number[]): Promise<void> {
  return invoke<void>("reorder_metadata_options", { fieldId, optionIds })
}

/** 선택지 삭제 요청 — 그 선택지를 가진 자산 수를 돌려준다 (값은 백그라운드에서 지워지고 재색인) */
export function deleteMetadataOption(optionId: number): Promise<number> {
  return invoke<number>("delete_metadata_option", { optionId })
}

/** 이 선택지를 값으로 가진 자산 수 (삭제 확인창) */
export function metadataOptionUsage(optionId: number): Promise<number> {
  return invoke<number>("metadata_option_usage", { optionId })
}

/** 자산 하나의 값 — 값이 있는 필드만 */
export function getAssetMetadata(assetId: number): Promise<AssetMetadataValue[]> {
  return invoke<AssetMetadataValue[]>("get_asset_metadata", { assetId })
}

/** 자산 하나의 값 지정 — patches 에 없는 필드는 그대로, 하나라도 잘못되면 전체 거부 */
export function setAssetMetadata(assetId: number, patches: MetadataPatch[]): Promise<void> {
  return invoke<void>("set_asset_metadata", { assetId, patches })
}

/** 여러 자산에 같은 패치 적용 (자산 최대 10,000건) — 값이 실제로 바뀐 자산 수를 돌려준다 */
export function bulkSetAssetMetadata(assetIds: number[], patches: MetadataPatch[]): Promise<number> {
  return invoke<number>("bulk_set_asset_metadata", { assetIds, patches })
}
