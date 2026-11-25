import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { useApi } from "@/lib/api"
import { isCommandError } from "@/lib/errors"
import { usePlaybackStore } from "@/stores"
import { qk } from "./keys"
import { useToastError } from "./useUserData"

/**
 * 재생 중인 컬렉션의 대기열을 계속 최신으로 — 순서 변경·항목 추가/제거 뒤 컬렉션 캐시가 무효화되면 다시 받아
 * 스토어에 반영한다(현재 항목 위치 재계산). 컬렉션이 사라졌으면(`not_found`) 재생을 멈춘다.
 */
export function usePlayQueue(collectionId: number | null) {
  const api = useApi()
  const query = useQuery({
    queryKey: qk.playQueue(collectionId ?? -1),
    queryFn: () => api.collectionPlayQueue(collectionId ?? -1),
    enabled: collectionId !== null,
    staleTime: 0,
  })
  const { data, error } = query
  useEffect(() => {
    if (data) usePlaybackStore.getState().setQueue(data)
  }, [data])
  useEffect(() => {
    if (isCommandError(error) && error.code === "not_found") usePlaybackStore.getState().stop()
  }, [error])
  return query
}

/** 컬렉션 연속 재생 시작 — 대기열을 받은 뒤 스토어에 넘긴다. 대기열 조회 실패는 toast 로 알린다. */
export function useStartPlayback() {
  const api = useApi()
  const qc = useQueryClient()
  const toastError = useToastError()
  return async (collectionId: number, fromAssetId?: number): Promise<void> => {
    try {
      const queue = await qc.fetchQuery({
        queryKey: qk.playQueue(collectionId),
        queryFn: () => api.collectionPlayQueue(collectionId),
        staleTime: 0,
      })
      usePlaybackStore.getState().start(collectionId, queue, fromAssetId)
    } catch (e) {
      toastError(e)
    }
  }
}
