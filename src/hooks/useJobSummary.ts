import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { useApi } from "@/lib/api"
import type { JobSummary } from "@/lib/ipc-contract"
import { qk } from "./keys"

/** 작업 요약 조회 + `jobs://progress` 구독으로 캐시를 갱신한다. */
export function useJobSummary() {
  const api = useApi()
  const qc = useQueryClient()
  const query = useQuery({ queryKey: qk.jobs, queryFn: () => api.jobSummary() })

  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null
    void api
      .onJobsProgress((s: JobSummary) => qc.setQueryData(qk.jobs, s))
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [api, qc])

  return query
}
