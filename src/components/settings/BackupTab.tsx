import { DatabaseBackup, ExternalLink, FolderOpen } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { useAppPaths, useBackupNow, useBackups, useUpdateSettings } from "@/hooks/useSettings"
import { useApi } from "@/lib/api"
import { errorKey, isCommandError } from "@/lib/errors"
import { formatBytes, formatDateTime } from "@/lib/format"
import type { BackupInfo, Settings } from "@/lib/ipc-contract"
import { useId, useState } from "react"
import { NumberField } from "./NumberField"
import { RestoreBackupDialog } from "./RestoreBackupDialog"
import { SettingRow } from "./SettingRow"
import { SettingsSubsection, SubsectionHeading } from "./SettingsSubsection"

function BackupDir({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const api = useApi()
  const update = useUpdateSettings()
  const paths = useAppPaths()
  const [error, setError] = useState<string | null>(null)
  const dir = settings.backup.dir

  const save = async (value: string | null) => {
    setError(null)
    try {
      await update.mutateAsync({ backup: { dir: value } })
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }
  const change = async () => {
    try {
      const picked = await api.pickDirectory()
      if (picked !== null) await save(picked)
    } catch (e) {
      setError(t(errorKey(e)))
    }
  }

  const open = async () => {
    setError(null)
    try {
      await api.openBackupDir()
    } catch (e) {
      setError(isCommandError(e) && e.code === "not_found" ? t("settings.backup.dirMissing") : t(errorKey(e)))
    }
  }

  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-sm font-medium">{t("settings.backup.dir")}</span>
      <p className="text-sm break-all">
        {dir === null ? t("settings.backup.dirDefault", { path: paths.data?.backupsDir ?? "" }) : dir}
      </p>
      <div className="flex gap-2">
        <Button size="sm" variant="outline" aria-label={t("settings.backup.dirChangeLabel")} onClick={() => void change()}>
          <FolderOpen aria-hidden="true" />
          {t("settings.backup.dirChange")}
        </Button>
        <Button size="sm" variant="outline" onClick={() => void open()}>
          <ExternalLink aria-hidden="true" />
          {t("settings.backup.dirOpen")}
        </Button>
        <Button size="sm" variant="ghost" disabled={dir === null} onClick={() => void save(null)}>
          {t("settings.backup.dirReset")}
        </Button>
      </div>
      {error && (
        <p role="alert" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </div>
  )
}

function BackupList() {
  const { t } = useTranslation()
  const backups = useBackups()
  const [target, setTarget] = useState<BackupInfo | null>(null)
  if (!backups.data || backups.data.length === 0) {
    return <p className="text-sm text-muted-foreground">{t("settings.backup.empty")}</p>
  }
  return (
    <>
      <ul aria-label={t("settings.backup.list")} className="flex flex-col divide-y divide-border rounded-lg border border-border">
        {backups.data.map((b) => (
          <li key={b.fileName} className="flex items-center gap-3 px-3 py-1.5 text-sm">
            <span className="tabular-nums">{formatDateTime(b.createdAt)}</span>
            <span className="text-muted-foreground">{t(`settings.backup.kind.${b.kind}`)}</span>
            <span className="ml-auto text-xs text-muted-foreground tabular-nums">{formatBytes(b.sizeBytes)}</span>
            <Button
              size="sm"
              variant="outline"
              aria-label={t("settings.backup.restore.buttonLabel", { date: formatDateTime(b.createdAt) })}
              onClick={() => setTarget(b)}
            >
              {t("settings.backup.restore.button")}
            </Button>
          </li>
        ))}
      </ul>
      <RestoreBackupDialog backup={target} onClose={() => setTarget(null)} />
    </>
  )
}

/** 설정 > 데이터·백업 > 자동 백업: 사용·주기·보관 개수·저장 위치. */
export function BackupSettingsSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const { backup } = settings

  return (
    <SettingsSubsection title={t("settings.sub.autoBackup")}>
      <SettingRow label={t("settings.backup.auto")} description={t("settings.backup.autoHint")}>
        <Switch
          aria-label={t("settings.backup.auto")}
          checked={backup.enabled}
          onCheckedChange={(enabled) => update.mutate({ backup: { enabled } })}
        />
      </SettingRow>
      <div className="flex flex-wrap gap-6">
        <NumberField
          label={t("settings.backup.interval")}
          unit={t("settings.unit.hours")}
          value={backup.intervalHours}
          min={1}
          max={720}
          onCommit={(intervalHours) => update.mutateAsync({ backup: { intervalHours } })}
        />
        <NumberField
          label={t("settings.backup.keep")}
          unit={t("settings.unit.items")}
          value={backup.keep}
          min={1}
          max={365}
          onCommit={(keep) => update.mutateAsync({ backup: { keep } })}
        />
      </div>
      <BackupDir settings={settings} />
    </SettingsSubsection>
  )
}

/** 설정 > 데이터·백업 > 백업 목록·복원: 지금 백업 + 목록(복원 버튼). */
export function BackupListSection() {
  const { t } = useTranslation()
  const now = useBackupNow()
  const headingId = useId()

  return (
    <section aria-labelledby={headingId} className="flex flex-col gap-4">
      <div className="flex items-start gap-2">
        <SubsectionHeading id={headingId} title={t("settings.backup.list")} />
        <Button size="sm" variant="outline" className="ml-auto" disabled={now.isPending} onClick={() => now.mutate()}>
          <DatabaseBackup aria-hidden="true" />
          {t("settings.backup.now")}
        </Button>
      </div>
      {now.isError && (
        <p role="alert" className="text-xs text-destructive">
          {t(errorKey(now.error))}
        </p>
      )}
      <BackupList />
    </section>
  )
}
