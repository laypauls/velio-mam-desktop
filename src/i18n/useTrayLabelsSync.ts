import { useEffect } from "react"
import { useApi } from "@/lib/api"
import type { TrayLabels } from "@/lib/ipc-contract"
import i18n from "./index"

function buildTrayLabels(): TrayLabels {
  return {
    open: i18n.t("tray.open"),
    pauseJobs: i18n.t("tray.pauseJobs"),
    resumeJobs: i18n.t("tray.resumeJobs"),
    quit: i18n.t("tray.quit"),
    aiWaiting: i18n.t("tray.aiWaiting", { app: i18n.t("app.title") }),
  }
}

/**
 * 앱 셸에서 한 번 호출 — 트레이 메뉴 문구를 표시 언어에 맞춰 코어에 넘긴다(시작 시 + 언어가 바뀔 때마다).
 * 문구는 꾸밈 요소라 실패해도 알리지 않는다(다음 언어 변경 때 다시 시도).
 */
export function useTrayLabelsSync(): void {
  const api = useApi()
  useEffect(() => {
    const push = () => {
      api.setTrayLabels(buildTrayLabels()).catch(() => {})
    }
    push()
    i18n.on("languageChanged", push)
    return () => {
      i18n.off("languageChanged", push)
    }
  }, [api])
}
