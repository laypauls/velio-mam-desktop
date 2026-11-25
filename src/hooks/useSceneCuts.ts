import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { CutSource, SceneCuts, SceneDetectOptions } from "@/lib/ipc-contract"
import { qk } from "./keys"

/**
 * 한 영상의 장면전환점 조회 + `scene://updated` 구독 — 추출이 끝나면 다시 읽는다(재생 중단 없음).
 * 이벤트가 없는 동안 놓칠 수 있으므로(보기를 닫은 사이 완료) 다시 마운트될 때는 항상 재조회한다.
 * `enabled=false`(영상이 아님) 면 조회·구독하지 않는다.
 */
export function useSceneCuts(assetId: number, enabled: boolean) {
  const api = useApi()
  const qc = useQueryClient()
  const query = useQuery({
    queryKey: qk.sceneCuts(assetId),
    queryFn: () => api.sceneCutsGet(assetId),
    enabled,
    staleTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
  })
  useEffect(() => {
    if (!enabled) return
    let disposed = false
    let unlisten: (() => void) | null = null
    void api
      .onSceneUpdated((id) => {
        if (id === assetId) void qc.invalidateQueries({ queryKey: qk.sceneCuts(assetId) })
      })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [api, qc, assetId, enabled])
  return query
}

export type SceneEdit =
  | { type: "add"; ms: number }
  | { type: "remove"; ms: number; source: CutSource }
  | { type: "clearAuto" }

/**
 * 컷 편집(추가·제거·자동 컷 모두 삭제) — 서버가 정규화한 전체 결과로 캐시를 바로 바꾼다.
 * 추가가 허용 오차 안의 기존 컷과 겹쳐 서버가 아무것도 바꾸지 않았으면(개수 동일) "이미 있음" 안내를 보인다.
 */
export function useSceneEdit(assetId: number) {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: (edit: SceneEdit): Promise<SceneCuts> => {
      switch (edit.type) {
        case "add":
          return api.sceneCutAdd(assetId, Math.round(edit.ms))
        case "remove":
          return api.sceneCutRemove(assetId, Math.round(edit.ms), edit.source)
        case "clearAuto":
          return api.sceneCutsClearAuto(assetId)
      }
    },
    onSuccess: (saved, edit) => {
      const before = qc.getQueryData<SceneCuts>(qk.sceneCuts(assetId))
      qc.setQueryData(qk.sceneCuts(assetId), saved)
      if (edit.type === "add" && before && before.cuts.length === saved.cuts.length) toast.info(t("scene.alreadyExists"))
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 장면 검출 등록 — 영상만 대기열에 올라가며, 등록 직후 해당 자산의 상태(pending)를 다시 읽는다 */
export function useSceneDetect() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: ({ assetIds, options }: { assetIds: number[]; options: SceneDetectOptions }) => api.sceneDetectRequest(assetIds, options),
    onSuccess: (res) => {
      void qc.invalidateQueries({ queryKey: qk.sceneCutsAll })
      void qc.invalidateQueries({ queryKey: qk.sceneBackfillCount })
      void qc.invalidateQueries({ queryKey: qk.jobs })
      if (res.queued === 0) toast.info(t("scene.requestNone"))
      else toast.success(t("scene.requested", { count: res.queued }))
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 설정 "모두 추출" — 아직 추출하지 않은 영상 전부를 낮은 우선순위로 등록 */
export function useSceneBackfill() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: () => api.sceneDetectBackfill(),
    onSuccess: (res) => {
      void qc.invalidateQueries({ queryKey: qk.sceneCutsAll })
      void qc.invalidateQueries({ queryKey: qk.sceneBackfillCount })
      void qc.invalidateQueries({ queryKey: qk.jobs })
      toast.success(t("scene.backfillDone", { count: formatNumber(res.queued) }))
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 백필 대상 영상 수 — 설정 화면이 열려 있을 때만 조회한다 */
export function useSceneBackfillCount() {
  const api = useApi()
  return useQuery({ queryKey: qk.sceneBackfillCount, queryFn: () => api.sceneBackfillCount(), staleTime: 0, retry: false })
}
