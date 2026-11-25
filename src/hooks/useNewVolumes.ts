import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { PendingVolume } from "@/lib/ipc-contract"
import { qk } from "./keys"

/**
 * 새 디스크 대기 목록 — 초기 조회 + `volumes://pending-changed`(전체 목록). 이벤트를 먼저 구독하고,
 * 조회 응답보다 이벤트가 먼저 오면 더 최신이므로 조회 결과로 덮어쓰지 않는다.
 */
export function usePendingVolumes(): PendingVolume[] {
  const api = useApi()
  const [volumes, setVolumes] = useState<PendingVolume[]>([])
  useEffect(() => {
    let alive = true
    let sawEvent = false
    let off: (() => void) | null = null
    void api
      .onPendingVolumesChanged((next) => {
        sawEvent = true
        if (alive) setVolumes(next)
      })
      .then((unlisten) => {
        if (alive) off = unlisten
        else unlisten()
      })
      .then(() => api.listPendingNewVolumes())
      .then((list) => {
        if (alive && !sawEvent) setVolumes(list)
      })
      .catch(() => undefined) // 조회 실패는 다음 이벤트가 목록을 채운다
    return () => {
      alive = false
      off?.()
    }
  }, [api])
  return volumes
}

/** "나중에" — 하나 또는 남은 전체. 실패는 toast (카드가 사라지지 않는다) */
export function useDismissVolumes() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: ({ volumeKeys, remember }: { volumeKeys: string[]; remember: boolean }) =>
      Promise.all(volumeKeys.map((k) => api.dismissNewVolume(k, remember))),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.knownVolumes }),
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 설정 "기억하는 디스크" 목록 — 새 디스크 대기 목록이 바뀔 때(`volumes://pending-changed`)도 다시 받는다 */
export function useKnownVolumes() {
  const api = useApi()
  const qc = useQueryClient()
  useEffect(() => {
    let alive = true
    let off: (() => void) | null = null
    void api
      .onPendingVolumesChanged(() => void qc.invalidateQueries({ queryKey: qk.knownVolumes }))
      .then((unlisten) => {
        if (alive) off = unlisten
        else unlisten()
      })
      .catch(() => undefined)
    return () => {
      alive = false
      off?.()
    }
  }, [api, qc])
  return useQuery({ queryKey: qk.knownVolumes, queryFn: () => api.listKnownVolumes() })
}

/** 다시 묻기 — 끝나면 목록을 다시 받는다 */
export function useUnignoreVolume() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: (volumeKey: string) => api.unignoreVolume(volumeKey),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.knownVolumes }),
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 기록 지우기 — 오류(`volume_has_folders` 등)는 확인 Dialog 가 mutation.error 로 인라인 표시한다 */
export function useForgetVolume() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (volumeKey: string) => api.forgetVolumeRecord(volumeKey),
    onSettled: () => void qc.invalidateQueries({ queryKey: qk.knownVolumes }),
  })
}
