import { Separator } from "@/components/ui/separator"
import type { Settings } from "@/lib/ipc-contract"
import { BackupListSection, BackupSettingsSection } from "./BackupTab"
import { DataLocationSection, DataResetZone } from "./DataTab"

/** 설정 > 데이터·백업: 자동 백업 / 백업 목록·복원 / 관리 데이터 위치 / 위험 영역(관리 데이터 초기화). */
export function DataBackupTab({ settings }: { settings: Settings }) {
  return (
    <div className="flex flex-col gap-4">
      <BackupSettingsSection settings={settings} />
      <Separator />
      <BackupListSection />
      <Separator />
      <DataLocationSection settings={settings} />
      <DataResetZone />
    </div>
  )
}
