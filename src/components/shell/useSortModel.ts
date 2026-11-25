import { useEffect } from "react"
import { useSettings } from "@/hooks/useSettings"
import type { SortField } from "@/lib/ipc-contract"
import { SEMANTIC_SORT, isSemanticActive, requestSort, useSearchStore } from "@/stores"

const SORT_FIELDS: SortField[] = ["relevance", "capturedAt", "modifiedAt", "name", "size", "duration", "rating"]

/** 설정에서 켠 정렬 항목 — 관련도는 항상 포함. 설정을 아직 못 받았으면 전부 보여 준다 */
function availableFields(enabled: Partial<Record<SortField, boolean>> | undefined, inCollection: boolean): SortField[] {
  const fields = SORT_FIELDS.filter((f) => f === "relevance" || enabled === undefined || enabled[f])
  // 컬렉션 순서는 설정의 노출 항목과 무관하게 컬렉션 보기에서만 고를 수 있다
  return inCollection ? [...fields, "collectionOrder"] : fields
}

/**
 * 정렬 선택지·상태·동작·비활성 규칙 — 상단 바 `SortControl` 과 "더보기" 메뉴가 같이 쓴다(로직 중복 방지).
 * `syncFallback` 은 현재 정렬 항목이 꺼졌을 때 대체 항목으로 전환하는 부수효과 — 한 곳(SortControl)에서만 켠다.
 */
export function useSortModel(syncFallback = false) {
  const userSort = useSearchStore((s) => s.sort)
  const setSort = useSearchStore((s) => s.setSort)
  const semantic = useSearchStore((s) => isSemanticActive(s.mode, s.query))
  const view = useSearchStore((s) => s.view)
  const filters = useSearchStore((s) => s.filters)
  const fields = availableFields(useSettings().data?.sortFieldsEnabled, view.kind === "collection")
  // 현재 정렬 항목이 꺼지면 켜진 첫 항목(없으면 관련도)으로 전환한다. 의미 검색 중에도 사용자 정렬은 보존되므로 userSort 로 판단한다
  const fallback = fields.find((f) => f !== "relevance") ?? "relevance"
  const missing = !fields.includes(userSort.field)
  useEffect(() => {
    if (syncFallback && missing) setSort({ ...userSort, field: fallback })
  }, [syncFallback, missing, fallback, userSort, setSort])
  // 의미 검색은 관련도순 고정 — 표시만 바꾸고 사용자가 고른 정렬은 스토어에 보존한다
  const sort = semantic ? SEMANTIC_SORT : requestSort(view, filters, userSort)
  // 컬렉션 순서는 오름차순 고정 — 방향 버튼을 끈다
  const orderPinned = sort.field === "collectionOrder"
  const desc = sort.direction === "desc"
  const onField = (f: string) => setSort(f === "collectionOrder" ? { field: "collectionOrder", direction: "asc" } : { ...userSort, field: f as SortField })
  const toggleDirection = () => setSort({ ...userSort, direction: userSort.direction === "desc" ? "asc" : "desc" })
  return { fields, sort, semantic, orderPinned, desc, onField, toggleDirection }
}
