import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import { formatNumber } from "@/lib/format"
import { qk } from "./keys"

/** 설정 > 이어보기: 관리 폴더별·전체 기록 수. 화면을 열 때마다 새로 읽는다 (재생 중 저장으로 수가 바뀐다) */
export function useResumeStats() {
  const api = useApi()
  return useQuery({ queryKey: qk.resumeStats, queryFn: () => api.resumeStats(), staleTime: 0, gcTime: 0 })
}

/** 선택 자산 중 이어보기 기록이 있는 수 — 메뉴 항목 활성화 판정. `enabled` 가 false 면 조회하지 않는다(이미지만 선택 등) */
export function useResumeCount(ids: readonly number[], enabled: boolean) {
  const api = useApi()
  const sorted = [...ids].sort((a, b) => a - b)
  return useQuery({
    queryKey: qk.resumeCount(sorted),
    queryFn: () => api.resumeCountAssets(sorted),
    enabled: enabled && sorted.length > 0,
    staleTime: 0,
    gcTime: 0,
  })
}

/** 기록 삭제 뒤 수를 다시 읽도록 이어보기 캐시를 비운다 */
function useInvalidateResume() {
  const qc = useQueryClient()
  return () => qc.invalidateQueries({ queryKey: qk.resumeAll })
}

/**
 * 선택한 콘텐츠의 이어보기 기록 삭제 (D9 — 확인 없이 즉시, 삭제 건수 toast).
 * 메뉴 항목은 실행과 동시에 닫혀 언마운트되므로 `mutate` 호출 단위 콜백은 버려진다 — 실패 toast 도 여기(훅 단위)에서 띄운다.
 */
export function useClearResumeAssets() {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateResume()
  return useMutation({
    mutationFn: (assetIds: number[]) => api.resumeClearAssets(assetIds),
    onSuccess: (count) => {
      toast.success(t("settings.resume.cleared", { count: formatNumber(count) }))
      void invalidate()
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })
}

/** 설정 > 이어보기의 폴더별 사용 토글 — 성공하면 stats 갱신, 실패하면 toast (Switch 는 stats 로 제어되므로 되돌릴 상태가 없다) */
export function useSetResumeFolderEnabled() {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateResume()
  return useMutation({
    mutationFn: (v: { folderId: number; enabled: boolean }) => api.resumeSetFolderEnabled(v.folderId, v.enabled),
    onSuccess: () => void invalidate(),
    onError: () => void toast.error(t("settings.resume.toggleFailed")),
  })
}

/** 설정 > 이어보기의 폴더별·전체 초기화 — 성공하면 건수 toast + 기록 수 갱신 */
export function useClearResumeScope() {
  const api = useApi()
  const { t } = useTranslation()
  const invalidate = useInvalidateResume()
  return useMutation({
    mutationFn: (scope: { kind: "folder"; folderId: number } | { kind: "all" }) => (scope.kind === "folder" ? api.resumeClearFolder(scope.folderId) : api.resumeClearAll()),
    onSuccess: (count) => {
      toast.success(t("settings.resume.cleared", { count: formatNumber(count) }))
      void invalidate()
    },
  })
}
