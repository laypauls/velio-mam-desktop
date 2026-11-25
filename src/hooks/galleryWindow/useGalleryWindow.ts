import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query"
import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { isCommandError, isSearchNotReady } from "@/lib/errors"
import type { AssetSummary, SearchIndex, SearchRequest, SortField } from "@/lib/ipc-contract"
import { DEFAULT_CAPTURED_AT_FALLBACK } from "@/lib/capturedAt"
import { INDEX_RETRY_MS, useSearchRequest } from "../useSearchAssets"
import { useFolders } from "../useLibrary"
import { useSettings } from "../useSettings"
import { qk } from "../keys"
import {
  BlockCache,
  INFINITE_BLOCK_SIZE,
  INFINITE_MAX_BLOCKS,
  PAGED_MAX_BLOCKS,
  neededBlocks,
  type PlannedRequest,
} from "./blockCache"

/** 빠른 스크롤 중 요청을 미루는 정지 시간 */
export const FAST_IDLE_MS = 120

export interface WindowTimers {
  set(fn: () => void, ms: number): unknown
  clear(handle: unknown): void
}
const realTimers: WindowTimers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
}

export interface UseGalleryWindowOptions {
  mode: "infinite" | "paged"
  /** 페이지 모드의 쪽 크기 (50|100|200) — 무한 스크롤에서는 블록 크기가 100 으로 고정이라 쓰지 않는다 */
  pageSize: number
  /** 페이지 모드의 0-기준 쪽 번호. 범위를 넘으면 마지막 쪽으로 맞춘다 */
  page?: number
  /** 보이는 블록 앞뒤로 더 받을 블록 수 (무한 스크롤, 기본 1) */
  overscanBlocks?: number
  fastIdleMs?: number
  indexRetryMs?: number
  timers?: WindowTimers
}

export interface RequestRangeOptions {
  /** true 면 `fastIdleMs` 동안 더 호출이 없을 때 한 번만 요청 */
  fast: boolean
}

export interface GalleryWindow {
  index: SearchIndex | undefined
  total: number
  generation: number | null
  getItem: (g: number) => AssetSummary | undefined
  /** 보관 중인 assetId 의 전역 위치 (없으면 null) */
  positionOf: (assetId: number) => number | null
  /** 보관 중인 항목의 assetId (위치 순) — 선택 범위 계산용. `version` 이 바뀔 때만 다시 만든다 */
  loadedIds: () => number[]
  /** 블록 캐시가 바뀔 때마다 증가 — 파생 값을 메모할 때 의존성으로 쓴다 */
  version: number
  loadedCount: number
  /** 화면에 보이는 전역 항목 구간 [firstItem, lastItem](포함). 페이지 모드에서는 쪽 조각만 받으므로 무시된다 */
  requestRange: (firstItem: number, lastItem: number, opts?: RequestRangeOptions) => void
  /** 보여 줄 전역 범위 [start, end) — 무한: [0,total) / 페이지: 그 쪽 */
  range: [number, number]
  /** 페이지 모드: 실제 쪽 번호(범위 보정 후)와 전체 쪽 수. 무한 스크롤은 0 / 1 */
  page: number
  pageCount: number
  /** 아직 보여 줄 데이터가 전혀 없음 (색인 대기 또는 첫 블록 대기) */
  isLoading: boolean
  /** 질의가 바뀌어 새 색인을 기다리는 동안 이전 결과를 보여 주는 중 */
  isRefreshing: boolean
  isError: boolean
  error: unknown
  indexPreparing: boolean
  degraded: boolean
  totalIsEstimate: boolean
  request: Omit<SearchRequest, "limit" | "cursor">
  /** 현재 `getItem` 이 반환하는 항목을 정렬한 필드 — 월 머리글 기준 (placeholder 중에는 이전 값 유지) */
  shownSortField: SortField
  /** 오류 후 다시 시도 — 색인 조회 실패면 색인부터, 블록 오류면 블록 요청부터 */
  retry: () => void
}

export function useGalleryWindow(opts: UseGalleryWindowOptions): GalleryWindow {
  const { mode, pageSize, overscanBlocks = 1, fastIdleMs = FAST_IDLE_MS, indexRetryMs = INDEX_RETRY_MS } = opts
  const timers = opts.timers ?? realTimers
  const api = useApi()
  const { t } = useTranslation()
  const qc = useQueryClient()
  const base = useSearchRequest()
  // 촬영일 대체 설정은 요청에 실리지 않고 백엔드가 직접 읽는다 — 바뀌면 정렬·구간이 달라지므로 색인 키에 넣어 다시 받고 블록 캐시를 비운다
  const fallback = useSettings().data?.capturedAtFallback ?? DEFAULT_CAPTURED_AT_FALLBACK
  // 숨긴 관리 폴더 목록도 백엔드가 직접 읽는다(Tantivy 세대는 그대로) — 바뀌면 옛 커서가 거부되므로 색인 키에 넣어 블록 캐시를 깨끗이 비운다
  const hiddenKey = useFolders().data?.filter((f) => f.hidden).map((f) => f.folderId).sort((a, b) => a - b).join(",") ?? ""
  const blockSize = mode === "infinite" ? INFINITE_BLOCK_SIZE : pageSize
  const maxBlocks = mode === "infinite" ? INFINITE_MAX_BLOCKS : PAGED_MAX_BLOCKS

  const indexKey = useMemo(() => [...qk.search, "index", base, fallback, hiddenKey] as const, [base, fallback, hiddenKey])
  const indexQuery = useQuery({
    queryKey: indexKey,
    queryFn: () => api.searchIndex(base),
    placeholderData: keepPreviousData,
    refetchInterval: (q) => (isSearchNotReady(q.state.error) ? indexRetryMs : false),
  })
  const index = indexQuery.data
  const ready = index !== undefined && !indexQuery.isPlaceholderData

  // 블록 크기·모드가 바뀌면 새 캐시 (모드 전환 시 비움)
  const cache = useMemo(() => new BlockCache({ blockSize, maxBlocks }), [blockSize, maxBlocks])
  const [version, bump] = useReducer((n: number) => n + 1, 0)
  const [blockError, setBlockError] = useState<unknown>(null)
  const blockErrorRef = useRef<unknown>(null)
  const wantedRef = useRef<[number, number]>([0, 0])
  const appliedBase = useRef(base)
  const appliedFallback = useRef(fallback)
  const appliedHidden = useRef(hiddenKey)
  const fastTimer = useRef<unknown>(null)
  const retryTimer = useRef<unknown>(null)
  const pumpRef = useRef<() => void>(() => {})
  /** 세대 변경 감지 뒤 새 색인이 오기 전까지 블록 요청을 보류 */
  const awaitingIndex = useRef(false)

  const total = index?.total ?? 0
  const pageCount = mode === "paged" ? Math.max(1, Math.ceil(total / pageSize)) : 1
  const page = mode === "paged" ? Math.min(Math.max(0, opts.page ?? 0), pageCount - 1) : 0
  const range: [number, number] = useMemo(
    () => (mode === "paged" ? [page * pageSize, Math.min(total, (page + 1) * pageSize)] : [0, total]),
    [mode, page, pageSize, total],
  )

  const fail = useCallback(
    (err: unknown) => {
      blockErrorRef.current = err
      setBlockError(err)
    },
    [],
  )
  const clearError = useCallback(() => {
    blockErrorRef.current = null
    setBlockError(null)
  }, [])

  /** 캐시를 비우고 색인을 다시 받는다 — 세대 변경·위치 어긋남·거부된 커서 */
  const invalidate = useCallback(() => {
    cache.reset()
    clearError()
    awaitingIndex.current = true
    bump()
    void qc.invalidateQueries({ queryKey: indexKey })
  }, [cache, clearError, qc, indexKey])

  const run = useCallback(
    (req: PlannedRequest) => {
      const params: SearchRequest = {
        ...base,
        limit: blockSize,
        cursor: req.cursor,
        ...(req.kind === "prev" ? { direction: "backward" as const } : {}),
        ...(req.kind === "at" ? { at: req.at } : {}),
      }
      api.searchAssets(params).then(
        (res) => {
          try {
            const r = cache.accept(req, res)
            if (r === "stale") return
            if (r === "generation") return invalidate()
            bump()
            if (!blockErrorRef.current) pumpRef.current()
          } catch (err) {
            // 응답 형식이 계약과 다르면 조용히 삼키지 않고 오류로 표면화한다
            console.error("search_assets response could not be applied", err)
            cache.fail(req)
            fail(err)
          }
        },
        (err: unknown) => {
          if (cache.currentEpoch !== req.epoch) return
          cache.fail(req)
          if (isSearchNotReady(err)) {
            if (retryTimer.current !== null) timers.clear(retryTimer.current)
            retryTimer.current = timers.set(() => pumpRef.current(), indexRetryMs)
          } else if (isCommandError(err) && err.code === "invalid_input" && req.kind !== "at") {
            // 색인 재구축·모델 교체로 옛 커서가 거부됨 — 한 번 비우고 색인부터 다시 (seek 요청 실패는 반복하지 않고 오류 표시)
            toast.info(t("search.cursorReset"))
            invalidate()
          } else fail(err)
        },
      )
    },
    [api, base, blockSize, cache, fail, indexRetryMs, invalidate, t, timers],
  )

  const pump = useCallback(() => {
    if (!ready || !index || awaitingIndex.current) return
    let needed: number[]
    let prefetch: number[] = []
    if (mode === "paged") {
      needed = [page]
      prefetch = [page + 1]
    } else {
      needed = neededBlocks(wantedRef.current[0], wantedRef.current[1], blockSize, index.total, overscanBlocks)
    }
    cache.setNeeded(needed)
    for (const req of cache.plan(prefetch, index.total)) run(req)
  }, [ready, index, mode, page, blockSize, overscanBlocks, cache, run])
  pumpRef.current = pump

  // 새 색인이 도착: 질의가 바뀌었으면 캐시를 비우고 맨 위로, 세대가 바뀌었으면 비운 뒤 이어서 요청
  useLayoutEffect(() => {
    if (!ready || !index) return
    awaitingIndex.current = false
    if (appliedBase.current !== base || appliedFallback.current !== fallback || appliedHidden.current !== hiddenKey) {
      appliedBase.current = base
      appliedFallback.current = fallback
      appliedHidden.current = hiddenKey
      cache.reset()
      wantedRef.current = [0, 0]
      clearError()
    }
    // 세대가 같은데 색인을 다시 받았다면(편집·분석 반영 후 무효화) 보관 블록은 그대로 보이되 다시 받게 표시한다
    if (!cache.setGeneration(index.generation)) cache.markStale()
    bump()
  }, [ready, index, indexQuery.dataUpdatedAt, base, fallback, hiddenKey, cache, clearError])

  useEffect(() => {
    pump()
  }, [pump, indexQuery.dataUpdatedAt])

  useEffect(
    () => () => {
      if (fastTimer.current !== null) timers.clear(fastTimer.current)
      if (retryTimer.current !== null) timers.clear(retryTimer.current)
    },
    [timers],
  )

  const requestRange = useCallback(
    (firstItem: number, lastItem: number, o?: RequestRangeOptions) => {
      wantedRef.current = [firstItem, lastItem]
      if (mode === "paged") return
      if (fastTimer.current !== null) timers.clear(fastTimer.current)
      fastTimer.current = null
      if (o?.fast) {
        fastTimer.current = timers.set(() => {
          fastTimer.current = null
          pumpRef.current()
        }, fastIdleMs)
      } else if (!blockErrorRef.current) pumpRef.current()
    },
    [mode, timers, fastIdleMs],
  )

  const { isError: indexFailed, refetch: refetchIndex } = indexQuery
  const retry = useCallback(() => {
    clearError()
    // 색인 조회 자체가 실패했으면 색인부터 다시 받는다 (받으면 블록 요청이 이어진다)
    if (indexFailed) void refetchIndex()
    else pumpRef.current()
  }, [clearError, indexFailed, refetchIndex])

  // 화면에 보이는 항목을 만든 정렬 필드 — placeholder 동안 이전 값 유지 (useSearchAssets 와 같은 의미)
  const [shownSortField, setShownSortField] = useState<SortField>(base.sort.field)
  if (!indexQuery.isPlaceholderData && shownSortField !== base.sort.field) setShownSortField(base.sort.field)

  const getItem = useCallback((g: number) => cache.getItem(g), [cache, version])
  const positionOf = useCallback((id: number) => cache.positionOf(id), [cache])
  const loadedIds = useCallback(() => cache.loadedIds(), [cache])
  const error = blockError ?? (indexQuery.isError ? indexQuery.error : null)
  const isError = error !== null
  const loadedCount = cache.loadedCount()
  return {
    index,
    total,
    generation: index?.generation ?? null,
    getItem,
    positionOf,
    loadedIds,
    version,
    loadedCount,
    requestRange,
    range,
    page,
    pageCount,
    isLoading: indexQuery.isPending || (total > 0 && loadedCount === 0 && !isError),
    isRefreshing: indexQuery.isPlaceholderData,
    isError,
    error,
    indexPreparing: indexQuery.isError && isSearchNotReady(indexQuery.error),
    degraded: cache.meta.degraded,
    totalIsEstimate: cache.meta.totalIsEstimate,
    request: base,
    shownSortField,
    retry,
  }
}
