import { useCallback } from "react"
import { useTranslation } from "react-i18next"
import { toast } from "sonner"
import { useApi } from "@/lib/api"
import { updateErrorCode } from "@/lib/update-errors"
import { useUpdateStore } from "@/stores/update"

const TOAST_ID = "update-available"

/** 업데이트 확인·설치 동작. 상태는 `useUpdateStore` (세션 단위). 설치는 `install()` 호출(=사용자 승인) 때만 일어난다. */
export function useUpdater() {
  const api = useApi()
  const { t } = useTranslation()

  /** `manual=false`(시작 시 자동 확인)는 실패해도 아무것도 표시하지 않는다. */
  const check = useCallback(
    async (manual: boolean) => {
      const s = useUpdateStore.getState()
      if (s.checking || s.phase !== "idle") return
      s.set({ checking: true, checkError: null })
      try {
        const res = await api.checkUpdate()
        if (!res.available || !res.version) {
          useUpdateStore.getState().set({ available: null, upToDate: true, checking: false })
          return
        }
        useUpdateStore.getState().set({ available: res, upToDate: false, checking: false })
        const cur = useUpdateStore.getState()
        if (cur.dismissedVersion !== res.version && cur.notifiedVersion !== res.version) {
          cur.set({ notifiedVersion: res.version })
          toast(t("update.toast.title", { version: res.version }), {
            id: TOAST_ID,
            description: t("update.toast.description"),
            action: { label: t("update.toast.view"), onClick: () => useUpdateStore.getState().set({ dialogOpen: true }) },
          })
        }
      } catch (e) {
        useUpdateStore.getState().set({ checking: false, checkError: manual ? (updateErrorCode(e) ?? "unknown") : null })
      }
    },
    [api, t],
  )

  const open = useCallback(() => useUpdateStore.getState().set({ dialogOpen: true, installError: null }), [])

  /** "나중에" — 이번 세션에서는 같은 버전을 다시 토스트하지 않는다. 설치 중에는 닫을 수 없다. */
  const dismiss = useCallback(() => {
    const s = useUpdateStore.getState()
    if (s.phase !== "idle") return
    s.set({ dialogOpen: false, dismissedVersion: s.available?.version ?? s.dismissedVersion })
  }, [])

  const install = useCallback(async () => {
    const s = useUpdateStore.getState()
    if (s.phase !== "idle") return
    s.set({ phase: "downloading", downloaded: 0, total: null, installError: null })
    let unlisten: (() => void) | null = null
    try {
      unlisten = await api.onUpdateState((p) => {
        if (p.phase === "failed") return // 원인은 command reject(UpdateError) 로 도착한다
        useUpdateStore.getState().set({ phase: p.phase, downloaded: p.downloaded, total: p.total })
      })
      await api.installUpdate()
    } catch (e) {
      useUpdateStore.getState().set({ phase: "idle", installError: updateErrorCode(e) ?? "unknown" })
    } finally {
      unlisten?.()
    }
  }, [api])

  /** 재시도 — 직전 확인 결과가 없으면(`not_checked`) 다시 확인한 뒤 설치한다. */
  const retry = useCallback(async () => {
    if (useUpdateStore.getState().installError === "not_checked") {
      useUpdateStore.getState().set({ installError: null })
      await check(true)
      if (!useUpdateStore.getState().available) return
    }
    await install()
  }, [check, install])

  return { check, open, dismiss, install, retry }
}
