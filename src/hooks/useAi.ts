import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { useApi } from "@/lib/api"
import type { AiSettingsPatch, AnalysisEdit, LocalModel } from "@/lib/ipc-contract-ai"
import { dayKey } from "@/lib/format"
import { analysisPollInterval } from "./useAnalysisEvents"
import { qk } from "./keys"

export const SIMILAR_PAGE = 24

export function useAiSettings() {
  const api = useApi()
  return useQuery({ queryKey: qk.aiSettings, queryFn: () => api.getAiSettings() })
}

/** 설정 저장 — 성공하면 캐시를 갱신한다. 실패는 호출부가 인라인으로 처리한다. */
export function useUpdateAiSettings() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (patch: AiSettingsPatch) => api.updateAiSettings(patch),
    onSuccess: (settings) => qc.setQueryData(qk.aiSettings, settings),
  })
}

export function useRevokeConsent() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: () => api.revokeAiConsent(),
    onSuccess: (settings) => {
      qc.setQueryData(qk.aiSettings, settings)
      void qc.invalidateQueries({ queryKey: qk.assetAiAll })
    },
  })
}

export function useProviders() {
  const api = useApi()
  return useQuery({ queryKey: qk.aiProviders, queryFn: () => api.listAiProviders() })
}

/** 로컬 모델 목록. `models://progress` 이벤트로 진행률을 갱신하고, 완료·실패 시 목록을 다시 읽는다. */
export function useLocalModels() {
  const api = useApi()
  const qc = useQueryClient()
  useEffect(() => {
    let unlisten: (() => void) | undefined
    let disposed = false
    void api
      .onModelsProgress((p) => {
        qc.setQueryData<LocalModel[]>(qk.aiModels, (old) =>
          old?.map((m) =>
            m.modelId === p.modelId ? { ...m, state: p.state, progress: p.total > 0 ? p.downloaded / p.total : null } : m,
          ),
        )
        if (p.state === "ready" || p.state === "failed") void qc.invalidateQueries({ queryKey: qk.aiModels })
      })
      .then((fn) => (disposed ? fn() : (unlisten = fn)))
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [api, qc])
  return useQuery({ queryKey: qk.aiModels, queryFn: () => api.listLocalModels() })
}

export function useAiUsage(days: number) {
  const api = useApi()
  return useQuery({
    queryKey: qk.aiUsage(days),
    queryFn: () => {
      const now = Date.now()
      return api.listAiUsage(dayKey(now - (days - 1) * 86_400_000), dayKey(now))
    },
  })
}

export function useAssetAi(assetId: number | null) {
  const api = useApi()
  return useQuery({
    queryKey: qk.assetAi(assetId),
    queryFn: () => api.getAssetAi(assetId as number),
    enabled: assetId !== null,
    // 이벤트(ai://analysis)가 주 경로 — 폴링은 이벤트가 없는 경로(대량 등록·동의 철회)와 이벤트 중단 시의 대비
    refetchInterval: (q) =>
      analysisPollInterval(q.state.data?.status.some((s) => s.state === "queued" || s.state === "running") ?? false, Date.now()),
  })
}

export function useUpdateAnalysis(assetId: number) {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (edit: AnalysisEdit) => api.updateAnalysis(assetId, edit),
    onSuccess: () => qc.invalidateQueries({ queryKey: qk.assetAi(assetId) }),
  })
}

export function useAnalysisHistory(assetId: number, enabled: boolean) {
  const api = useApi()
  return useQuery({ queryKey: qk.analysisHistory(assetId), queryFn: () => api.listAnalysisHistory(assetId), enabled })
}

interface SimilarParam {
  cursor: string | null
  /** 지금까지 받은 개수 — 다음 요청 limit 을 남은 개수로 줄이는 데 쓴다 */
  loaded: number
}

/**
 * 유사 미디어 — 누적 개수가 `max`(설정 최대 표시 개수)에 닿으면 다음 페이지가 없는 것으로 본다.
 * `max`·`minSimilarity` 는 쿼리 키에 포함돼 설정이 바뀌면 처음부터 다시 조회한다(하한은 백엔드가 적용).
 */
export function useSimilar(assetId: number, max: number, minSimilarity: number, enabled = true) {
  const api = useApi()
  return useInfiniteQuery({
    queryKey: [...qk.similar(assetId), max, minSimilarity] as const,
    enabled,
    queryFn: ({ pageParam }) =>
      api.findSimilar({ assetId, limit: Math.min(SIMILAR_PAGE, max - pageParam.loaded), cursor: pageParam.cursor }),
    initialPageParam: { cursor: null, loaded: 0 } as SimilarParam,
    getNextPageParam: (last, all): SimilarParam | undefined => {
      const loaded = all.reduce((n, p) => n + p.items.length, 0)
      return last.nextCursor !== null && loaded < max ? { cursor: last.nextCursor, loaded } : undefined
    },
  })
}

/** 용도 접두(`e5`, `clip`)에 해당하는 모델이 하나 이상 있고 모두 준비됐는지 */
export function purposeReady(models: LocalModel[], prefix: string): boolean {
  const group = models.filter((m) => m.purpose.startsWith(prefix))
  return group.length > 0 && group.every((m) => m.state === "ready")
}

/**
 * 모델 준비 상태 — 의미(하이브리드) 검색은 e5 텍스트 모델만, 유사 미디어·시각 임베딩은 CLIP 모델이 필요하다.
 * `loaded` 가 false 인 동안(목록 조회 전)은 "없음"으로 단정하지 않는다.
 */
export function useModelReadiness() {
  const list = useLocalModels().data
  return {
    loaded: list !== undefined,
    e5Ready: list ? purposeReady(list, "e5") : false,
    clipReady: list ? purposeReady(list, "clip") : false,
    clipModels: (list ?? []).filter((m) => m.purpose.startsWith("clip")),
  }
}
