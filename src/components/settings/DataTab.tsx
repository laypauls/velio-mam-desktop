import { FolderOpen, RotateCcw } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { useAppPaths, useUpdateSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import { errorKey } from "@/lib/errors"
import type { Settings } from "@/lib/ipc-contract"
import { useProtectStore } from "@/stores"
import { DangerZone } from "./DangerZone"
import { SettingsSubsection } from "./SettingsSubsection"

/** PL11: 네트워크(UNC) 경로는 관리 데이터 위치로 쓸 수 없다. */
export function isNetworkPath(path: string): boolean {
  return path.startsWith("\\\\")
}

/** 설정 > 데이터·백업 > 관리 데이터 위치: 위치 변경(네트워크 경로 거부). 적용은 재시작 후. */
export function DataLocationSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const api = useApi()
  const update = useUpdateSettings()
  const paths = useAppPaths()
  const [error, setError] = useState<string | null>(null)
  const pendingRestart = paths.data !== undefined && paths.data.dataDir !== settings.dataDir

  const change = async () => {
    setError(null)
    try {
      const dir = await api.pickDirectory()
      if (dir === null) return
      if (isNetworkPath(dir)) {
        setError(t("settings.data.networkRejected"))
        return
      }
      await update.mutateAsync({ dataDir: dir })
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  return (
    <SettingsSubsection title={t("settings.data.location")}>
      <div className="flex flex-col gap-1.5">
        <p className="text-sm break-all">{settings.dataDir}</p>
        <p className="text-xs text-muted-foreground break-keep">
          {pendingRestart ? t("settings.data.pendingRestart", { path: paths.data?.dataDir ?? "" }) : t("settings.data.hint")}
        </p>
        <Button size="sm" variant="outline" className="self-start" aria-label={t("settings.data.changeLabel")} onClick={() => void change()}>
          <FolderOpen aria-hidden="true" />
          {t("settings.data.change")}
        </Button>
        {error && (
          <p role="alert" className="text-xs text-destructive">
            {error}
          </p>
        )}
      </div>
    </SettingsSubsection>
  )
}

/** 설정 > 데이터·백업 > 위험 영역: 관리 데이터 초기화(비밀번호 인증은 초기화 다이얼로그에서). 원본 파일은 지우지 않는다. */
export function DataResetZone() {
  const { t } = useTranslation()
  const setResetOpen = useProtectStore((s) => s.setResetOpen)
  return (
    <DangerZone>
      <div className="flex flex-col gap-1.5">
        <span className="text-sm font-medium">{t("settings.data.reset")}</span>
        <p className="text-xs text-muted-foreground break-keep">{t("settings.data.resetHint")}</p>
        <Button size="sm" variant="destructive" className="self-start" onClick={() => setResetOpen(true)}>
          <RotateCcw aria-hidden="true" />
          {t("settings.data.reset")}
        </Button>
      </div>
    </DangerZone>
  )
}
