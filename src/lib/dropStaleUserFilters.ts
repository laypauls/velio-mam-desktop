import { toast } from "sonner"
import i18n from "@/i18n"
import type { MetadataField } from "@/lib/ipc-contract-metadata"
import { pruneUserFields, userFieldsOrUndefined } from "@/lib/userFieldFilters"
import { useSearchStore } from "@/stores"

/** 빠진 사용자 필드 조건이 있음을 알린다 */
export function notifyDroppedUserFilters(count: number): void {
  toast.info(i18n.t("metadata.filter.dropped", { count }))
}

/**
 * 현재 필터에서 없어진 필드·선택지를 쓰는 사용자 필드 조건을 빼고, 뺀 경우 안내한다. 뺀 조건 수를 돌려준다.
 * 낡은 조건으로 검색하면 백엔드가 `invalid_input` 을 돌려주므로, 필드·선택지 삭제 직후와 필드 목록이 바뀔 때마다 부른다.
 */
export function dropStaleUserFilters(fields: MetadataField[]): number {
  const { filters, patchFilters } = useSearchStore.getState()
  if (!filters.userFields?.length) return 0
  const { kept, dropped } = pruneUserFields(filters.userFields, fields)
  if (dropped === 0) return 0
  patchFilters({ userFields: userFieldsOrUndefined(kept) })
  notifyDroppedUserFilters(dropped)
  return dropped
}
