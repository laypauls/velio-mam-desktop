import { useMemo } from "react"
import type { SearchRequest } from "@/lib/ipc-contract"
import { SEMANTIC_SORT, effectiveFilters, isSemanticActive, requestSort, useSearchStore } from "@/stores"

/** 색인 준비 중(`index_not_ready`) 응답 뒤 다시 묻는 간격 */
export const INDEX_RETRY_MS = 1500

type BaseRequest = Omit<SearchRequest, "limit" | "cursor">

/** 스토어의 질의·필터·정렬·사이드바 보기를 합친 검색 요청 (limit·cursor 제외). 갤러리 창(`useGalleryWindow`)이 쓴다. */
export function useSearchRequest(): BaseRequest {
  const view = useSearchStore((s) => s.view)
  const query = useSearchStore((s) => s.query)
  const filters = useSearchStore((s) => s.filters)
  const userSort = useSearchStore((s) => s.sort)
  const mode = useSearchStore((s) => s.mode)
  // 의미 모드는 질의가 있을 때만 요청에 싣는다 (빈 질의는 목록 조회)
  const semantic = isSemanticActive(mode, query)
  // 의미 검색은 관련도순만 가능 — 사용자가 고른 정렬은 스토어에 그대로 두고 요청에서만 덮어쓴다
  // 컬렉션 순서는 컬렉션 보기에서만 유효 — 저장한 검색 등으로 밖에서 들어오면 기본 정렬로 돌린다
  const sort = semantic ? SEMANTIC_SORT : requestSort(view, filters, userSort)
  // deps 에는 요청에 실제로 실리는 값만 둔다 — `mode` 는 semantic 일 때만 실리므로 semantic 으로 대신한다
  // (빈 질의에서 모드만 토글해도 새 객체가 만들어져 갤러리 캐시가 초기화되던 문제 방지)
  return useMemo(
    () => ({
      query,
      filters: effectiveFilters(view, filters),
      sort,
      ...(semantic ? { mode: "hybrid" as const } : {}),
    }),
    [view, query, filters, sort, semantic],
  )
}
