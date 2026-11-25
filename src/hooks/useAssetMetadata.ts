import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { bulkSetAssetMetadata, getAssetMetadata, setAssetMetadata } from "@/lib/ipc-metadata"
import type { MetadataPatch } from "@/lib/ipc-contract-metadata"
import { qk } from "./keys"

/** 자산 하나의 사용자 메타데이터 값 — `qk.assets` 접두 아래라 자산 변경 시 함께 무효화된다. */
export function useAssetMetadata(assetId: number) {
  return useQuery({
    queryKey: [...qk.assets, "metadata", assetId] as const,
    queryFn: () => getAssetMetadata(assetId),
  })
}

/** 값이 바뀐 뒤 갱신할 캐시 — 검색 결과(필터·키워드 적중이 바뀜)와 상세. */
function useInvalidateMetadataValues() {
  const qc = useQueryClient()
  return () =>
    Promise.all([qc.invalidateQueries({ queryKey: qk.search }), qc.invalidateQueries({ queryKey: qk.assets })])
}

export function useSetAssetMetadata() {
  const invalidate = useInvalidateMetadataValues()
  return useMutation({
    mutationFn: (v: { assetId: number; patches: MetadataPatch[] }) => setAssetMetadata(v.assetId, v.patches),
    onSuccess: invalidate,
  })
}

/** 여러 자산에 같은 패치 적용 — 값이 바뀐 자산 수를 돌려준다. */
export function useBulkSetAssetMetadata() {
  const invalidate = useInvalidateMetadataValues()
  return useMutation({
    mutationFn: (v: { assetIds: number[]; patches: MetadataPatch[] }) => bulkSetAssetMetadata(v.assetIds, v.patches),
    onSuccess: invalidate,
  })
}
