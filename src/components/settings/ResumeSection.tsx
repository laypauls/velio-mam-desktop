import { useState } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select"
import { Separator } from "@/components/ui/separator"
import { Switch } from "@/components/ui/switch"
import { useFolders } from "@/hooks/useLibrary"
import { useResumeStats, useSetResumeFolderEnabled } from "@/hooks/useResume"
import { useUpdateSettings } from "@/hooks/useSettings"
import { folderDisplayName } from "@/lib/folders"
import { formatNumber } from "@/lib/format"
import { RESUME_MIN_DURATION_CHOICES, type Settings } from "@/lib/ipc-contract"
import { ResumeFolderTable } from "./ResumeFolderTable"
import { ResumeResetDialog, type ResumeResetTarget } from "./ResumeResetDialog"
import { SettingRow } from "./SettingRow"
import { SettingsSubsection } from "./SettingsSubsection"

/** 설정 > "이어보기" 페이지 본문(제목은 페이지 헤더): 동작(사용·최소 길이) / 관리 폴더별 기록 / 전체. 초기화는 확인 Dialog 를 거친다 */
export function ResumeSection({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const update = useUpdateSettings()
  const folders = useFolders()
  const stats = useResumeStats()
  const [target, setTarget] = useState<ResumeResetTarget | null>(null)
  const counts = new Map((stats.data?.folders ?? []).map((f) => [f.folderId, f.count]))
  const enabledMap = new Map((stats.data?.folders ?? []).map((f) => [f.folderId, f.enabled]))
  const setFolderEnabled = useSetResumeFolderEnabled()
  const total = stats.data?.total ?? 0
  // Radix SelectItem 은 빈 문자열 값을 쓸 수 없다 — 값은 초 단위 숫자 문자열("0" 포함)
  const minLabel = (sec: number) => (sec === 0 ? t("settings.resume.minNone") : t("settings.resume.minOption", { minutes: sec / 60 }))

  return (
    <section aria-label={t("settings.resume.section")} className="flex flex-col gap-4">
      <SettingsSubsection title={t("settings.resume.sub.behavior")}>
        <SettingRow label={t("settings.resume.enabled")} description={t("settings.resume.enabledHint")}>
          <Switch aria-label={t("settings.resume.enabled")} checked={settings.resumeEnabled} onCheckedChange={(resumeEnabled) => update.mutate({ resumeEnabled })} />
        </SettingRow>
        <SettingRow label={t("settings.resume.minDuration")} description={t("settings.resume.minDurationHint")}>
          <Select value={String(settings.resumeMinDurationSec)} onValueChange={(v) => update.mutate({ resumeMinDurationSec: Number(v) })}>
            <SelectTrigger aria-label={t("settings.resume.minDuration")} className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {RESUME_MIN_DURATION_CHOICES.map((sec) => (
                <SelectItem key={sec} value={String(sec)}>
                  {minLabel(sec)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </SettingRow>
      </SettingsSubsection>
      <Separator />
      <SettingsSubsection title={t("settings.resume.sub.folders")}>
        {folders.data !== undefined && folders.data.length === 0 && <p className="text-sm text-muted-foreground">{t("settings.resume.empty")}</p>}
        {folders.data !== undefined && folders.data.length > 0 && (
          <ResumeFolderTable
            folders={folders.data}
            counts={counts}
            enabled={enabledMap}
            switchesDisabled={!settings.resumeEnabled || stats.data === undefined}
            onToggle={(folder, enabled) => setFolderEnabled.mutate({ folderId: folder.folderId, enabled })}
            onReset={(folder, count) => setTarget({ kind: "folder", folderId: folder.folderId, name: folderDisplayName(folder), count })}
          />
        )}
        {stats.isError && (
          <p role="alert" className="text-xs text-destructive">
            {t("settings.resume.loadFailed")}
          </p>
        )}
      </SettingsSubsection>
      <Separator />
      <SettingsSubsection title={t("settings.resume.sub.all")}>
        <SettingRow label={t("settings.resume.totalLabel")} description={t("settings.resume.totalCount", { count: formatNumber(total) })}>
          <Button variant="outline" disabled={total === 0} onClick={() => setTarget({ kind: "all", count: total })}>
            {t("settings.resume.resetAll")}
          </Button>
        </SettingRow>
      </SettingsSubsection>
      <ResumeResetDialog target={target} onClose={() => setTarget(null)} />
    </section>
  )
}
