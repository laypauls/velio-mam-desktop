import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect, useRef, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import type { FolderRemovalProgress, MediaTypeChangeMode, MediaTypeSet } from "@/lib/ipc-contract"
import { qk } from "./keys"

const SCAN_ERROR_PAGE = 50

export function useInvalidateFolders() {
  const qc = useQueryClient()
  return () => {
    for (const key of [qk.folders, qk.search, qk.jobs, qk.scanErrors, qk.knownVolumes]) void qc.invalidateQueries({ queryKey: key })
  }
}

/** add_folder 호출. 오류는 Dialog 가 인라인으로 보여줄 수 있게 mutation.error 로 남긴다 (폴더 선택은 `useAddFolder`). */
export function useAddFolderRequest() {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()
  return useMutation({
    mutationFn: ({ path, mediaTypes }: { path: string; mediaTypes: MediaTypeSet }) => api.addFolder(path, mediaTypes),
    onSuccess: (folder) => {
      toast.success(t("folders.added", { path: folder.path }))
      invalidate()
    },
  })
}

/** 스캔 대상 유형 변경. 제외 유형 purge 는 목록·검색에서 빠지므로 자산 상세 캐시까지 무효화한다. 오류는 인라인. */
export function useSetFolderMediaTypes() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()
  return useMutation({
    mutationFn: ({ folderId, mediaTypes, mode }: { folderId: number; mediaTypes: MediaTypeSet; mode: MediaTypeChangeMode }) =>
      api.setFolderMediaTypes(folderId, mediaTypes, mode),
    onSuccess: (res) => {
      toast.success(res.detached > 0 ? t("settings.folders.typesPurged", { count: formatNumber(res.detached) }) : t("settings.folders.typesUpdated"))
      invalidate()
      void qc.invalidateQueries({ queryKey: qk.assets })
      void qc.invalidateQueries({ queryKey: ["folderTypeCounts"] })
    },
  })
}

/** 폴더의 유형별 활성 자산 수 — 열 때마다 새로 조회한다. */
export function useFolderAssetCounts(folderId: number) {
  const api = useApi()
  return useQuery({
    queryKey: ["folderTypeCounts", folderId],
    queryFn: () => api.countFolderAssetsByType(folderId),
    staleTime: 0,
    gcTime: 0,
  })
}

export function useRescanFolder() {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()
  return useMutation({
    mutationFn: (folderId: number) => api.rescanFolder(folderId),
    onSuccess: () => {
      toast.success(t("settings.folders.rescanStarted"))
      invalidate()
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 관리 폴더 경로 변경 — 오류는 toast 가 아니라 다이얼로그가 mutation.error 로 인라인 표시한다. */
export function useRelocateFolder() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()
  return useMutation({
    mutationFn: ({ folderId, newPath }: { folderId: number; newPath: string }) => api.relocateFolder(folderId, newPath),
    onSuccess: (res) => {
      toast.success(t("settings.folders.relocated", { path: res.folder.path, count: formatNumber(res.relocated) }))
      invalidate()
      void qc.invalidateQueries({ queryKey: qk.assets })
      void qc.invalidateQueries({ queryKey: ["subfolders", res.folder.folderId] })
    },
  })
}

/** 제거 확인 Dialog 용 — 지워질 연결 데이터 개수. 열 때마다 새로 조회하고 실패해도 제거는 막지 않는다. */
export function useFolderRemovalImpact(folderId: number | null) {
  const api = useApi()
  return useQuery({
    queryKey: qk.folderRemovalImpact(folderId ?? -1),
    queryFn: () => api.folderRemovalImpact(folderId as number),
    enabled: folderId !== null,
    staleTime: 0,
    gcTime: 0,
    retry: false,
  })
}

/**
 * 관리 폴더 제거 — 반환 시점에는 폴더가 목록에서 빠졌고 연결 데이터 삭제는 백그라운드로 이어진다
 * (진행률은 `useFolderRemovalProgress`). `backup_failed`·`busy` 는 Dialog 가 mutation.error 로 인라인 표시한다.
 */
export function useRemoveFolder() {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()
  return useMutation({
    mutationFn: (folderId: number) => api.removeFolder(folderId),
    onSuccess: () => {
      toast.success(t("settings.folders.removed"))
      invalidate()
    },
  })
}

/**
 * `folders://removal-progress` 구독 — 진행 중인 제거의 합산 진행률(없으면 null). 끝나면 목록·검색·기억하는 디스크를 다시 받고,
 * 오류로 끝나면 toast. 상태 표시줄이 쓴다.
 */
export function useFolderRemovalProgress(): { done: number; total: number } | null {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()
  const [active, setActive] = useState<ReadonlyMap<number, FolderRemovalProgress>>(new Map())
  const onFinished = useRef({ invalidate, t })
  onFinished.current = { invalidate, t }
  useEffect(() => {
    let alive = true
    let off: (() => void) | null = null
    void api
      .onFolderRemovalProgress((p) => {
        if (!alive) return
        setActive((prev) => {
          const next = new Map(prev)
          if (p.finished) next.delete(p.folderId)
          else next.set(p.folderId, p)
          return next
        })
        if (p.finished) {
          onFinished.current.invalidate()
          if (p.error !== null) toast.error(onFinished.current.t("settings.folders.removalFailed"))
        }
      })
      .then((unlisten) => {
        if (alive) off = unlisten
        else unlisten()
      })
      .catch(() => undefined)
    return () => {
      alive = false
      off?.()
    }
  }, [api])
  if (active.size === 0) return null
  let done = 0
  let total = 0
  for (const p of active.values()) {
    done += p.done
    total += p.total
  }
  return { done, total }
}

/** 스캔 오류 목록 — cursor 페이지 단위. */
export function useScanErrors(enabled: boolean) {
  const api = useApi()
  return useInfiniteQuery({
    queryKey: qk.scanErrors,
    queryFn: ({ pageParam }) => api.listScanErrors(pageParam, SCAN_ERROR_PAGE),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextCursor,
    enabled,
  })
}

/** 스캔 오류 재시도 — `errorIds` null 이면 전체. */
export function useRetryScanErrors() {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateFolders()
  return useMutation({
    mutationFn: (errorIds: number[] | null) => api.retryScanErrors(errorIds),
    onSuccess: (count) => {
      toast.success(t("settings.folders.retried", { count: formatNumber(count) }))
      invalidate()
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/**
 * 관리 폴더 숨김 토글. 숨김은 질의 시점 필터라 재색인이 없으므로, 폴더 목록과 모든 검색·갤러리 쿼리를 무효화하면 즉시 반영된다.
 * 오류는 호출부가 인라인으로 보여줄 수 있게 mutation.error 로 남긴다.
 */
export function useSetFolderHidden() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ folderId, hidden }: { folderId: number; hidden: boolean }) => api.setFolderHidden(folderId, hidden),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: qk.folders })
      // 검색 색인 키에 숨김 폴더 목록이 들어 있어 폴더 목록이 갱신되면 새 질의로 바뀐다 — 옛 키를 즉시 다시 받으면 옛 커서가 거부되므로 표시만 한다
      void qc.invalidateQueries({ queryKey: qk.search, refetchType: "none" })
      for (const key of [qk.ratingCounts, qk.similarAll, qk.jobs]) void qc.invalidateQueries({ queryKey: key })
    },
  })
}

/** 관리 폴더 별칭 지정·제거. 폴더 목록만 무효화하면 사이드바·설정이 즉시 갱신된다 (검색 결과에는 영향 없음). */
export function useSetFolderAlias() {
  const api = useApi()
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ folderId, alias }: { folderId: number; alias: string | null }) => api.setFolderAlias(folderId, alias),
    onSuccess: () => void qc.invalidateQueries({ queryKey: qk.folders }),
  })
}
