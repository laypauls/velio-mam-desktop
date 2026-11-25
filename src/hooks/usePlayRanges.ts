import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { mergeRanges, roundRanges } from "@/components/viewer/playRanges"
import { useApi } from "@/lib/api"
import type { PlayRange, PlayRangeSummary } from "@/lib/ipc-contract"
import { qk } from "./keys"

/** 편집 후 저장까지의 지연(ms) — 연속 편집(핸들 드래그 직후 키 조작 등)을 한 번의 전체 교체로 합친다 */
export const RANGE_SAVE_DEBOUNCE_MS = 300

const EMPTY: readonly PlayRange[] = []

export interface EditablePlayRanges {
  /** 화면에 보일 구간 — 저장 대기/진행 중이면 낙관적 값, 아니면 서버 값 */
  ranges: readonly PlayRange[]
  /** 서버에서 구간을 받았는가 (구간만 재생은 받기 전에는 판단하지 않는다) */
  loaded: boolean
  /** 전체 교체 예약 — 정수 ms 로 맞추고 정렬·병합한 값을 즉시 보이며, 서버 정규화 결과가 오면 그것으로 대체한다 */
  commit: (next: readonly PlayRange[]) => void
  /** 마지막 저장이 실패했는가 (이전 서버 값으로 되돌아간 상태) */
  saveFailed: boolean
}

/**
 * 한 자산의 재생 구간 조회 + 디바운스 저장. 저장은 직렬화(이전 요청이 끝난 뒤 최신 값)하고, 성공하면 서버가 병합한 값으로
 * 캐시를 갱신하며 컬렉션 요약을 무효화한다. 실패하면 낙관적 값을 버리고 이전 서버 값으로 돌아가 toast 로 알린다.
 * 언마운트(자산 전환·보기 닫기) 때 대기 중인 편집은 즉시 보낸다 — 편집을 잃지 않는다.
 */
export function useEditablePlayRanges(assetId: number): EditablePlayRanges {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  const query = useQuery({ queryKey: qk.playRanges(assetId), queryFn: () => api.playRangesGet(assetId), staleTime: Number.POSITIVE_INFINITY })
  const [draft, setDraft] = useState<readonly PlayRange[] | null>(null)
  const [saveFailed, setSaveFailed] = useState(false)
  const latest = useRef<PlayRange[] | null>(null)
  const version = useRef(0)
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const chain = useRef<Promise<void>>(Promise.resolve())

  const flush = useCallback(() => {
    timer.current = null
    const snapshot = latest.current
    if (snapshot === null) return
    const sent = version.current
    chain.current = chain.current.then(async () => {
      try {
        const saved = await api.playRangesSet(assetId, snapshot)
        qc.setQueryData(qk.playRanges(assetId), saved)
        void qc.invalidateQueries({ queryKey: qk.playRangeSummaryAll })
        if (version.current === sent) {
          latest.current = null
          setDraft(null)
        }
        setSaveFailed(false)
      } catch {
        toast.error(t("ranges.saveFailed"))
        setSaveFailed(true)
        if (version.current === sent) {
          latest.current = null
          setDraft(null)
        }
      }
    })
  }, [api, assetId, qc, t])

  const commit = useCallback(
    (next: readonly PlayRange[]) => {
      const merged = mergeRanges(roundRanges(next))
      latest.current = merged
      version.current += 1
      setDraft(merged)
      if (timer.current !== null) clearTimeout(timer.current)
      timer.current = setTimeout(flush, RANGE_SAVE_DEBOUNCE_MS)
    },
    [flush]
  )

  useEffect(
    () => () => {
      if (timer.current !== null) {
        clearTimeout(timer.current)
        flush()
      }
    },
    [flush]
  )

  return { ranges: draft ?? query.data ?? EMPTY, loaded: query.isSuccess || draft !== null, commit, saveFailed }
}

/**
 * 컬렉션 대기열의 구간 요약(구간 있는 항목만) — 재생 중인 컬렉션마다 1회 조회하고, 구간 편집 저장 때만 다시 받는다.
 * 실패해도 대기열 표시는 그대로 두므로 빈 맵.
 */
export function usePlayRangeSummary(collectionId: number | null): ReadonlyMap<number, PlayRangeSummary> {
  const api = useApi()
  const { data } = useQuery({
    queryKey: qk.playRangeSummary(collectionId ?? -1),
    queryFn: () => api.collectionPlayRangeSummary(collectionId ?? -1),
    enabled: collectionId !== null,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  })
  return useMemo(() => new Map((data ?? []).map((s) => [s.assetId, s] as const)), [data])
}
