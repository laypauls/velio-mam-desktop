import { keepPreviousData, useMutation, useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect, useState } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import {
  onPluginChanged,
  pluginInspectDir,
  pluginInstall,
  pluginList,
  pluginRetryFailed,
  pluginRunAll,
  pluginSetEnabled,
  pluginTaskCounts,
  pluginUninstall,
  type PluginPreview,
} from "@/lib/ipc-plugin"
import { useJobSummary } from "./useJobSummary"
import { qk } from "./keys"

/** 설치된 플러그인 목록. `plugin://changed`(설치·활성·권한·자동 비활성)를 구독해 목록과 작업 수를 다시 받는다. */
export function usePluginList() {
  const qc = useQueryClient()
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null
    onPluginChanged(() => {
      void qc.invalidateQueries({ queryKey: qk.plugins })
      void qc.invalidateQueries({ queryKey: qk.pluginCounts })
    })
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => undefined) // 이벤트를 못 받아도 목록은 열 때·변경 때 다시 받는다
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [qc])
  return useQuery({ queryKey: qk.plugins, queryFn: pluginList })
}

/** 플러그인별 대기·실패 수. 작업 요약(`jobs://progress`)의 plugin 종류 수가 바뀔 때만 다시 조회한다 — 별도 폴링 없음. */
export function usePluginCounts() {
  const jobs = useJobSummary()
  const c = jobs.data?.counts.plugin
  const signature = c ? `${c.pending}/${c.running}/${c.failed}` : ""
  return useQuery({ queryKey: [...qk.pluginCounts, signature], queryFn: pluginTaskCounts, placeholderData: keepPreviousData })
}

/** 목록·작업 수·작업 요약을 함께 갱신 */
function useRefreshPlugins() {
  const qc = useQueryClient()
  return () =>
    Promise.all([
      qc.invalidateQueries({ queryKey: qk.plugins }),
      qc.invalidateQueries({ queryKey: qk.pluginCounts }),
      qc.invalidateQueries({ queryKey: qk.jobs }),
    ])
}

export function useSetPluginEnabled() {
  const refresh = useRefreshPlugins()
  return useMutation({
    mutationFn: (v: { pluginId: string; enabled: boolean }) => pluginSetEnabled(v.pluginId, v.enabled),
    onSuccess: () => refresh(),
  })
}

export function useRunAllPlugin() {
  const { t } = useTranslation()
  const refresh = useRefreshPlugins()
  return useMutation({
    mutationFn: (v: { pluginId: string; providerId: string }) => pluginRunAll(v.pluginId, v.providerId),
    onSuccess: (r) => {
      toast.success(t(r.started ? "settings.plugins.runAllStarted" : "settings.plugins.runAllAlready"))
      return refresh()
    },
  })
}

export function useRetryPluginFailed() {
  const { t } = useTranslation()
  const refresh = useRefreshPlugins()
  return useMutation({
    mutationFn: (pluginId: string) => pluginRetryFailed(pluginId),
    onSuccess: (count) => {
      toast.success(t("settings.plugins.retried", { count }))
      return refresh()
    },
  })
}

export function useUninstallPlugin() {
  const { t } = useTranslation()
  const refresh = useRefreshPlugins()
  return useMutation({
    mutationFn: (v: { pluginId: string; keepData: boolean }) => pluginUninstall(v.pluginId, v.keepData),
    onSuccess: () => {
      toast.success(t("settings.plugins.uninstalled"))
      return refresh()
    },
  })
}

interface InstallSession {
  dir: string
  /** 검사가 실패하면 null — 오류는 `error` */
  preview: PluginPreview | null
  error: unknown
}

/**
 * 설치 흐름: 폴더 선택 → `plugin_inspect_dir` → (동의 Dialog) → `plugin_install`.
 * 선택 취소는 아무것도 하지 않는다. 폴더 선택 실패는 toast, 검사·설치 실패는 Dialog 안 인라인(+ toast 보조).
 * 동의한 권한 = 미리보기가 요청한 전체 권한(부분 동의 없음).
 */
export function usePluginInstall() {
  const api = useApi()
  const { t } = useTranslation()
  const refresh = useRefreshPlugins()
  const [session, setSession] = useState<InstallSession | null>(null)
  const [picking, setPicking] = useState(false)

  const install = useMutation({
    mutationFn: ({ dir, preview }: { dir: string; preview: PluginPreview }) =>
      pluginInstall(dir, preview.id, preview.version, preview.permissions),
    onSuccess: async (info) => {
      toast.success(t("settings.plugins.installed", { name: info.displayName }))
      setSession(null)
      await refresh()
    },
    onError: (e) => void toast.error(t(errorKey(e))),
  })

  const start = async () => {
    setPicking(true)
    try {
      let dir: string | null
      try {
        dir = await api.pickDirectory()
      } catch (e) {
        toast.error(t(errorKey(e)))
        return
      }
      if (dir === null) return
      install.reset()
      try {
        setSession({ dir, preview: await pluginInspectDir(dir), error: null })
      } catch (e) {
        setSession({ dir, preview: null, error: e })
        toast.error(t(errorKey(e)))
      }
    } finally {
      setPicking(false)
    }
  }

  return {
    session,
    picking,
    installing: install.isPending,
    installError: install.isError ? install.error : null,
    start,
    confirm: () => {
      if (session?.preview) install.mutate({ dir: session.dir, preview: session.preview })
    },
    close: () => {
      install.reset()
      setSession(null)
    },
  }
}

export type PluginInstallFlow = ReturnType<typeof usePluginInstall>
