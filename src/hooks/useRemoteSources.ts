import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useApi } from "@/lib/api"
import type { RemoteSourceInput } from "@/lib/ipc-contract"

const KEY = ["remoteSources"] as const

/** 저장된 원격 접속 목록 (이름 순) — 비밀은 들어 있지 않다. */
export function useRemoteSources() {
  const api = useApi()
  return useQuery({ queryKey: KEY, queryFn: () => api.listRemoteSources() })
}

/** 만들기(`sourceId` null) 또는 수정. 오류는 호출부가 필드 옆에 인라인으로 보여 준다. */
export function useSaveRemoteSource() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ sourceId, input }: { sourceId: number | null; input: RemoteSourceInput }) =>
      sourceId === null ? api.createRemoteSource(input) : api.updateRemoteSource(sourceId, input),
    onSuccess: () => qc.invalidateQueries({ queryKey: KEY }),
  })
}

/** 삭제 — 쓰는 폴더가 있으면 `busy` 로 거부된다. */
export function useDeleteRemoteSource() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (sourceId: number) => api.deleteRemoteSource(sourceId),
    onSuccess: () => qc.invalidateQueries({ queryKey: KEY }),
  })
}

/** 접속 시험 — 네트워크를 쓰므로 사용자가 누를 때만 호출한다 (캐시하지 않는다). */
export function useTestRemoteSource() {
  const api = useApi()
  return useMutation({
    mutationFn: ({ sourceId, input }: { sourceId: number | null; input: RemoteSourceInput }) => api.testRemoteSource(sourceId, input),
  })
}
