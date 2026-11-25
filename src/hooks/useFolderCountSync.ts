import { useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { useApi } from "@/lib/api"
import type { JobSummary } from "@/lib/ipc-contract"
import { qk } from "./keys"

/** 폴더 개수 재조회 모으기 — 첫 변화 후 이 시간 뒤 한 번만 (trailing, 마지막 변화는 항상 반영) */
export const FOLDER_COUNT_SYNC_MS = 2000

/** 스캔·감시로 폴더별 개수가 바뀌었을 가능성이 있는 전이인가 — 진행률만 바뀐 이벤트는 해당하지 않는다 */
export function folderCountsMayHaveChanged(prev: JobSummary, next: JobSummary): boolean {
  return prev.totalAssets !== next.totalAssets || (prev.scan !== null && next.scan === null)
}

/**
 * 앱 전체에서 한 번만 마운트한다(`useJobSummary` 는 Sidebar·StatusBar 두 곳에서 구독하므로 거기에 두지 않는다).
 * `jobs://progress` 에서 전체 자산 수가 바뀌거나 스캔이 끝나면, 사이드바 폴더 개수·하위 폴더 트리·
 * (열려 있는) 유형별 개수 질의를 스로틀해 다시 읽는다. 비활성 질의는 다시 읽지 않는다.
 */
export function useFolderCountSync(): void {
  const api = useApi()
  const qc = useQueryClient()
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null
    let timer: ReturnType<typeof setTimeout> | null = null
    let prev: JobSummary | undefined = qc.getQueryData<JobSummary>(qk.jobs)
    void api
      .onJobsProgress((next: JobSummary) => {
        const before = prev
        prev = next
        if (before === undefined || !folderCountsMayHaveChanged(before, next) || timer !== null) return
        timer = setTimeout(() => {
          timer = null
          void qc.invalidateQueries({ queryKey: qk.folders })
          void qc.invalidateQueries({ queryKey: ["subfolders"] })
          void qc.invalidateQueries({ queryKey: ["folderTypeCounts"] })
        }, FOLDER_COUNT_SYNC_MS)
      })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
    return () => {
      disposed = true
      unlisten?.()
      if (timer !== null) clearTimeout(timer)
    }
  }, [api, qc])
}
