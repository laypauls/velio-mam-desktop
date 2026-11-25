import { useMutation, useQueryClient } from "@tanstack/react-query"
import { toast } from "sonner"
import { useTranslation } from "react-i18next"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { UpdateTagsInput } from "@/lib/ipc-contract"
import { usePlaybackStore, useSearchStore } from "@/stores"
import { qk } from "./keys"

/** 실패 알림은 인라인이 우선이고 toast 는 보조 — 인라인 자리가 없는 호출부가 쓴다. */
export function useToastError(): (e: unknown) => void {
  const { t } = useTranslation()
  return (e) => void toast.error(t(errorKey(e)))
}

/** 자산 변경(태그·메모·즐겨찾기·컬렉션 구성) 뒤 갱신할 캐시: 검색 페이지·상세·태그·컬렉션 개수. */
function useInvalidateAssets() {
  const qc = useQueryClient()
  return () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: qk.search }),
      qc.invalidateQueries({ queryKey: qk.assets }),
      qc.invalidateQueries({ queryKey: qk.tags }),
      qc.invalidateQueries({ queryKey: qk.collections }),
    ])
}

export function useUpdateTags() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: (input: UpdateTagsInput) => api.updateAssetTags(input),
    onSuccess: invalidate,
  })
}

export function useSetMemo() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: ({ assetId, memo }: { assetId: number; memo: string }) => api.setMemo(assetId, memo),
    onSuccess: invalidate,
  })
}

export function useSetAssetTitle() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: ({ assetId, title }: { assetId: number; title: string | null }) => api.setAssetTitle(assetId, title),
    onSuccess: invalidate,
  })
}

export function useSetFavorite() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: ({ assetIds, favorite }: { assetIds: number[]; favorite: boolean }) => api.setFavorite(assetIds, favorite),
    onSuccess: invalidate,
  })
}

/** 별점 지정(0 = 해제) — 검색 결과·상세·사이드바 별점 개수를 다시 받는다. */
export function useSetRating() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ assetIds, rating }: { assetIds: number[]; rating: number }) => api.setRating(assetIds, rating),
    onSuccess: () => Promise.all([invalidate(), qc.invalidateQueries({ queryKey: qk.ratingCounts })]),
  })
}

export function useAddToCollection() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: ({ collectionId, assetIds }: { collectionId: number; assetIds: number[] }) =>
      api.addToCollection(collectionId, assetIds),
    onSuccess: invalidate,
  })
}

export function useRemoveFromCollection() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: ({ collectionId, assetIds }: { collectionId: number; assetIds: number[] }) =>
      api.removeFromCollection(collectionId, assetIds),
    onSuccess: invalidate,
  })
}

export function useCreateCollection() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: async ({ name, assetIds }: { name: string; assetIds: number[] }) => {
      const created = await api.createCollection(name)
      if (assetIds.length > 0) await api.addToCollection(created.collectionId, assetIds)
      return created
    },
    onSuccess: invalidate,
  })
}

export function useRenameCollection() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ collectionId, name }: { collectionId: number; name: string }) => api.renameCollection(collectionId, name),
    onSuccess: () => Promise.all([qc.invalidateQueries({ queryKey: qk.collections }), qc.invalidateQueries({ queryKey: qk.assets })]),
  })
}

/** 컬렉션만 지운다 — 자산(미디어 파일)은 그대로다. 현재 보고 있던 컬렉션이면 전체 보기로 돌아간다. */
export function useDeleteCollection() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: (collectionId: number) => api.deleteCollection(collectionId),
    onSuccess: async (_v, collectionId) => {
      const { view, setView } = useSearchStore.getState()
      if (view.kind === "collection" && view.collectionId === collectionId) setView({ kind: "all" })
      // 재생 중인 컬렉션이면 재생도 멈춘다 (보기 영역은 그대로 둔다)
      if (usePlaybackStore.getState().collectionId === collectionId) usePlaybackStore.getState().stop()
      await invalidate()
    },
  })
}

export function useRenameTag() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: ({ tagId, name }: { tagId: number; name: string }) => api.renameTag(tagId, name),
    onSuccess: invalidate,
  })
}

export function useDeleteTag() {
  const api = useApi()
  const invalidate = useInvalidateAssets()
  return useMutation({
    mutationFn: (tagId: number) => api.deleteTag(tagId),
    onSuccess: invalidate,
  })
}
