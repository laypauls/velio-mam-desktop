import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useApi } from "@/lib/api"
import type { RescanPolicy, WatchOptions } from "@/lib/ipc-contract"
import { qk } from "./keys"

/** 폴더 기준 사용 가능 감지기 — 열 때마다 새로 조회한다 (볼륨 상태에 따라 달라진다). */
export function useChangeDetectors(folderId: number | null) {
  const api = useApi()
  return useQuery({
    queryKey: ["changeDetectors", folderId],
    queryFn: () => api.listChangeDetectors(folderId as number),
    enabled: folderId !== null,
    staleTime: 0,
  })
}

/** 감지 방식 저장 — 폴더 목록(watchActive 등)을 다시 읽는다. 오류는 호출부가 인라인으로 처리한다. */
export function useSetFolderWatchMode() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ folderId, mode, options }: { folderId: number; mode: string | null; options: WatchOptions | null }) =>
      api.setFolderWatchMode(folderId, mode, options),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.folders }),
  })
}

/** 재스캔 정책 저장 (null = 전역 기본). */
export function useSetFolderRescanPolicy() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ folderId, policy }: { folderId: number; policy: RescanPolicy | null }) => api.setFolderRescanPolicy(folderId, policy),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.folders }),
  })
}
