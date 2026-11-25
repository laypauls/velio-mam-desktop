import { useEffect } from "react"
import { dropStaleUserFilters } from "@/lib/dropStaleUserFilters"
import { useSearchStore } from "@/stores"
import { useMetadataFields } from "./useMetadataFields"

/** 필드 목록이 갱신될 때(설정에서 삭제 등) 활성 필터의 낡은 조건을 자동으로 뺀다. 항상 마운트되는 AppShell 에서 쓴다. */
export function useUserFieldFilterSync(): void {
  const fields = useMetadataFields()
  const userFields = useSearchStore((s) => s.filters.userFields)
  useEffect(() => {
    // 목록을 정상 수신한 경우에만 — 로딩·실패 중에 비교하면 멀쩡한 조건을 지운다
    if (!fields.isSuccess || fields.isFetching || !userFields?.length) return
    dropStaleUserFilters(fields.data)
  }, [fields.isSuccess, fields.isFetching, fields.data, userFields])
}
