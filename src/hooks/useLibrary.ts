import { useQuery } from "@tanstack/react-query"
import { useApi } from "@/lib/api"
import { qk } from "./keys"

/** `refetchMs`: 주기적 갱신(상태 배지용) — 창이 가려져 있으면 멈춘다. 생략하면 무효화될 때만 다시 읽는다. */
export function useFolders(refetchMs?: number) {
  const api = useApi()
  return useQuery({ queryKey: qk.folders, queryFn: () => api.listFolders(), refetchInterval: refetchMs })
}

export function useSubfolders(folderId: number, relPath: string | null, enabled: boolean) {
  const api = useApi()
  return useQuery({
    queryKey: qk.subfolders(folderId, relPath),
    queryFn: () => api.listSubfolders(folderId, relPath),
    enabled,
  })
}

export function useTags() {
  const api = useApi()
  return useQuery({ queryKey: qk.tags, queryFn: () => api.listTags() })
}

export function useCollections() {
  const api = useApi()
  return useQuery({ queryKey: qk.collections, queryFn: () => api.listCollections() })
}

export function useRecentSearches() {
  const api = useApi()
  return useQuery({ queryKey: qk.recentSearches, queryFn: () => api.listRecentSearches() })
}
