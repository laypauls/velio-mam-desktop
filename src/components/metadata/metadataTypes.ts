import { useTranslation } from "react-i18next"
import type { MetadataValueType } from "@/lib/ipc-contract-metadata"

/** 필드 타입 표시 순서 (타입 Select 의 항목 순서) */
export const METADATA_VALUE_TYPES: readonly MetadataValueType[] = ["keyword", "number", "date", "boolean", "enum", "multiEnum"]

/** 선택지를 가지는 타입 (enum / multiEnum) */
export function hasOptions(type: MetadataValueType): boolean {
  return type === "enum" || type === "multiEnum"
}

/** 필드·선택지 이름 최대 글자 수 (Rust `MAX_NAME_CHARS`) */
export const METADATA_NAME_MAX = 100

/** 타입 라벨 번역 키 — `metadata.types.<type>` */
export function metadataTypeKey(type: MetadataValueType): string {
  return `metadata.types.${type}`
}

/** 타입 → 화면 이름 (짧은 글 / 숫자 / 날짜 / 예·아니요 / 선택 / 다중 선택) */
export function useMetadataTypeLabel(): (type: MetadataValueType) => string {
  const { t } = useTranslation()
  return (type) => t(metadataTypeKey(type))
}

/** 이름 중복 비교 키 — 백엔드와 같은 규칙(앞뒤 공백 제거 + NFC + 소문자) */
export function nameKey(raw: string): string {
  return raw.trim().normalize("NFC").toLowerCase()
}
