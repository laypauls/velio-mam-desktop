import { useQuery } from "@tanstack/react-query"
import { useApi } from "@/lib/api"
import { qk } from "./keys"

/** 사이드바 별점 개수(길이 6, index = 별점·0 = 미평가). `enabled` 가 false 면 질의하지 않는다. */
export function useRatingCounts(enabled: boolean) {
  const api = useApi()
  return useQuery({ queryKey: qk.ratingCounts, queryFn: () => api.ratingCounts(), enabled })
}
