import { FolderPlus } from "lucide-react"
import { useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import { AddFolderDialog } from "@/components/folders/AddFolderDialog"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
import { useAddFolder } from "@/hooks/useAddFolder"
import { useChangeDetectors } from "@/hooks/useFolderDetection"
import { useFolders } from "@/hooks/useLibrary"
import { useUpdateSettings } from "@/hooks/useSettings"
import type { Settings } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { FolderRow, NARROW_HIDDEN } from "./FolderRow"
import { FolderSettingsDialog } from "./folder-dialog/FolderSettingsDialog"
import { KnownVolumesSection } from "./KnownVolumesSection"
import { NumberField } from "./NumberField"
import { ScanErrorsSection } from "./ScanErrorsSection"
import { SettingsSubsection } from "./SettingsSubsection"

const FOLDERS_REFRESH_MS = 15_000

/** 설정 > 관리 폴더: 폴더 목록 / 스캔(재스캔 간격) / 스캔 오류. */
export function FoldersTab({ settings }: { settings: Settings }) {
  const { t } = useTranslation()
  const updateSettings = useUpdateSettings()
  // 상태 배지(신호원 상태·스캔 멈춤)가 낡지 않게 설정 화면이 열려 있는 동안만 주기적으로 다시 읽는다
  const folders = useFolders(FOLDERS_REFRESH_MS)
  // "실험" 표시는 감지기의 정적 속성이라 아무 폴더 기준 목록 하나로 충분하다
  const detectors = useChangeDetectors(folders.data?.[0]?.folderId ?? null)
  const experimental = useMemo(() => new Set((detectors.data ?? []).filter((d) => d.experimental).map((d) => d.id)), [detectors.data])
  const add = useAddFolder()
  const [settingsTarget, setSettingsTarget] = useState<number | null>(null)

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-center gap-2">
        <h3 className="text-sm font-semibold">{t("settings.folders.title")}</h3>
        <Button size="sm" variant="outline" className="ml-auto" disabled={add.isPending} onClick={() => add.start()}>
          <FolderPlus aria-hidden="true" />
          {t("settings.folders.add")}
        </Button>
      </div>
      {(folders.data?.length ?? 0) === 0 ? (
        <p className="text-sm text-muted-foreground">{t("sidebar.foldersEmpty")}</p>
      ) : (
        <div className="@container rounded-lg border border-border">
          <table aria-label={t("settings.folders.title")} className="w-full table-fixed border-collapse">
            <thead>
              <tr className="border-b border-border text-left text-xs text-muted-foreground">
                <th scope="col" className="px-2 py-1.5 font-medium">
                  {t("settings.folders.table.columns.folder")}
                </th>
                <th scope="col" className="w-20 px-2 py-1.5 font-medium">
                  <span className="block truncate">{t("settings.folders.table.columns.types")}</span>
                </th>
                <th scope="col" className={cn("w-44 px-2 py-1.5 font-medium", NARROW_HIDDEN)}>
                  <span className="block truncate">{t("settings.folders.table.columns.watch")}</span>
                </th>
                <th scope="col" className="w-40 px-2 py-1.5 font-medium">
                  <span className="block truncate">{t("settings.folders.table.columns.rescan")}</span>
                </th>
                <th scope="col" className={cn("w-20 px-2 py-1.5 text-right font-medium", NARROW_HIDDEN)}>
                  <span className="block truncate">{t("settings.folders.table.columns.assets")}</span>
                </th>
                <th scope="col" className="w-[4.5rem] px-2 py-1.5">
                  <span className="sr-only">{t("settings.folders.table.columns.actions")}</span>
                </th>
              </tr>
            </thead>
            <tbody>
              {folders.data?.map((f) => (
                <FolderRow key={f.folderId} folder={f} defaultHours={settings.rescanIntervalHours} onOpenSettings={() => setSettingsTarget(f.folderId)} experimental={experimental} />
              ))}
            </tbody>
          </table>
        </div>
      )}
      <KnownVolumesSection />
      <Separator />
      <SettingsSubsection title={t("settings.sub.scan")}>
        <NumberField
          label={t("settings.general.rescanInterval")}
          unit={t("settings.unit.hours")}
          value={settings.rescanIntervalHours}
          min={1}
          max={24 * 30}
          onCommit={(rescanIntervalHours) => updateSettings.mutateAsync({ rescanIntervalHours })}
        />
        <p className="text-xs text-muted-foreground break-keep">{t("settings.general.rescanIntervalHint")}</p>
        <NumberField
          label={t("settings.scan.maxConcurrent")}
          unit={t("settings.unit.items")}
          value={settings.maxConcurrentScans}
          min={1}
          max={8}
          onCommit={(maxConcurrentScans) => updateSettings.mutateAsync({ maxConcurrentScans })}
        />
        <p className="text-xs text-muted-foreground break-keep">{t("settings.scan.maxConcurrentHint")}</p>
      </SettingsSubsection>
      <Separator />
      <ScanErrorsSection />
      <FolderSettingsDialog folderId={settingsTarget} onClose={() => setSettingsTarget(null)} />
      <AddFolderDialog flow={add} />
    </div>
  )
}
