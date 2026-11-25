import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import type { SettingsPatch } from "@/lib/ipc-contract"
import { qk } from "./keys"

export function useSettings() {
  const api = useApi()
  return useQuery({ queryKey: qk.settings, queryFn: () => api.getSettings() })
}

export function useAppPaths() {
  const api = useApi()
  return useQuery({ queryKey: qk.appPaths, queryFn: () => api.appPaths() })
}

export function useBackups() {
  const api = useApi()
  return useQuery({ queryKey: qk.backups, queryFn: () => api.listBackups() })
}

/** 설정 저장. 성공하면 캐시를 갱신하고 저장 toast 를 띄운다 (`silent` 면 toast 생략). 실패는 호출부가 인라인으로 처리한다. */
export function useUpdateSettings({ silent = false }: { silent?: boolean } = {}) {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: (patch: SettingsPatch) => api.updateSettings(patch),
    onSuccess: (settings, patch) => {
      qc.setQueryData(qk.settings, settings)
      // 유사 미디어 기준이 바뀌면 열려 있는 유사 탭 조회를 다시 받는다
      // 촬영일 대체가 바뀌면 정렬·목차·필터 결과가 달라지므로 검색 캐시를 다시 받는다
      if ("capturedAtFallback" in patch) void qc.invalidateQueries({ queryKey: qk.search })
      if ("similarMaxResults" in patch || "similarMinSimilarity" in patch) void qc.invalidateQueries({ queryKey: qk.similarAll })
      if (!silent) toast.success(t("settings.saved"))
    },
  })
}

/** 지금 백업 — 성공하면 목록을 갱신한다. */
export function useBackupNow() {
  const api = useApi()
  const qc = useQueryClient()
  const { t } = useTranslation()
  return useMutation({
    mutationFn: () => api.backupNow(),
    onSuccess: () => {
      toast.success(t("settings.backup.done"))
      void qc.invalidateQueries({ queryKey: qk.backups })
    },
  })
}
