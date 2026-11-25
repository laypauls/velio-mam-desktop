import { useQuery, useQueryClient } from "@tanstack/react-query"
import { useEffect } from "react"
import { onPluginChanged, pluginContributions } from "@/lib/ipc-plugin"
import { qk } from "./keys"

/** 자산 메뉴의 플러그인 기여 항목 — 한 번 받아 캐시하고 `plugin://changed` 때만 다시 받는다 */
export function usePluginContributions() {
  return useQuery({
    queryKey: qk.pluginContributions,
    queryFn: pluginContributions,
    staleTime: Infinity,
    retry: false,
  })
}

/** 앱 전체에서 한 번만 마운트한다(AppShell) — 설치·활성·권한·자동 비활성 변경이 오면 기여 캐시를 비운다 */
export function usePluginContributionsSync(): void {
  const qc = useQueryClient()
  useEffect(() => {
    let disposed = false
    let unlisten: (() => void) | null = null
    onPluginChanged(() => void qc.invalidateQueries({ queryKey: qk.pluginContributions }))
      .then((fn) => {
        if (disposed) fn()
        else unlisten = fn
      })
      .catch(() => undefined) // 이벤트 채널이 없는 환경(웹 미리보기) — 메뉴는 첫 조회 값을 쓴다
    return () => {
      disposed = true
      unlisten?.()
    }
  }, [qc])
}
