import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useApi } from "@/lib/api"
import type { CollectionMembership, ReorderTarget } from "@/lib/ipc-contract"
import { qk } from "./keys"
import { useAddToCollection, useRemoveFromCollection, useToastError } from "./useUserData"

/** `collection_membership` 한 번에 보낼 수 있는 최대 자산 수 (백엔드 상한과 같다) */
export const MEMBERSHIP_MAX_IDS = 10_000

/**
 * 선택한 자산들의 컬렉션별 소속 개수. 메뉴가 열려 있는 동안만(`enabled`) 조회하고 캐시는 남기지 않는다.
 * 키는 정렬한 id 목록이라 선택 순서가 달라도 같은 질의다.
 */
export function useCollectionMembership(ids: readonly number[], enabled: boolean) {
  const api = useApi()
  const sorted = [...ids].sort((a, b) => a - b)
  return useQuery({
    queryKey: qk.membership(sorted),
    queryFn: () => api.collectionMembership(sorted),
    enabled: enabled && ids.length > 0 && ids.length <= MEMBERSHIP_MAX_IDS,
    staleTime: 0,
    gcTime: 0,
  })
}

/**
 * 메뉴 체크 토글 — 소속 캐시를 먼저 고쳐 체크가 바로 바뀌게 하고, 실패하면 toast 후 다시 받는다.
 * 성공 toast 는 없다(여러 컬렉션을 연달아 토글하는 흐름).
 */
export function useToggleMembership(ids: readonly number[]) {
  const qc = useQueryClient()
  const add = useAddToCollection()
  const remove = useRemoveFromCollection()
  const toastError = useToastError()
  const key = qk.membership([...ids].sort((a, b) => a - b))
  return (collectionId: number, removing: boolean) => {
    qc.setQueryData<CollectionMembership[]>(key, (old) => {
      const rest = (old ?? []).filter((m) => m.collectionId !== collectionId)
      return removing ? rest : [...rest, { collectionId, count: ids.length }]
    })
    const opts = {
      onError: (e: unknown) => {
        toastError(e)
        void qc.invalidateQueries({ queryKey: key })
      },
    }
    const input = { collectionId, assetIds: [...ids] }
    if (removing) remove.mutate(input, opts)
    else add.mutate(input, opts)
  }
}

/** 컬렉션 안 순서 바꾸기 — 순서만 바뀌므로 상세 캐시(`qk.assets`)는 건드리지 않고 검색 페이지와 컬렉션 대기열만 다시 받는다. */
export function useReorderCollection() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ collectionId, assetIds, target }: { collectionId: number; assetIds: number[]; target: ReorderTarget }) =>
      api.reorderCollection(collectionId, assetIds, target),
    onSuccess: () => Promise.all([qc.invalidateQueries({ queryKey: qk.search }), qc.invalidateQueries({ queryKey: qk.collections })]),
  })
}
