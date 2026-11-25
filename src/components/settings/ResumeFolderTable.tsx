import { EyeOff } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import { folderDisplayName } from "@/lib/folders"
import { formatNumber } from "@/lib/format"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { PathLine } from "./FolderRow"

interface Props {
  folders: readonly ManagedFolder[]
  /** 폴더 id → 기록 수 (없으면 0) */
  counts: ReadonlyMap<number, number>
  /** 폴더 id → 이어보기 사용 여부 (stats 에 없으면 켬) */
  enabled: ReadonlyMap<number, boolean>
  /** 사용 스위치 잠금 — 전역 이어보기가 꺼져 있거나 stats 를 아직 못 읽었을 때 (값은 보존) */
  switchesDisabled: boolean
  onToggle: (folder: ManagedFolder, enabled: boolean) => void
  onReset: (folder: ManagedFolder, count: number) => void
}

/** 관리 폴더별 이어보기 표 — 이름·경로 2줄(관리 폴더 표와 같은 `PathLine`), 사용 스위치, 기록 수, 초기화. 숨김 폴더도 포함하고, 기록 0건이면 초기화를 끈다 */
export function ResumeFolderTable({ folders, counts, enabled, switchesDisabled, onToggle, onReset }: Props) {
  const { t } = useTranslation()
  const head = "px-2 py-1.5 text-left text-xs font-medium text-muted-foreground"
  return (
    <div className="relative overflow-x-auto rounded-lg border border-border">
      <table aria-label={t("settings.resume.tableLabel")} className="w-full table-fixed">
        <thead className="border-b border-border bg-muted/40">
          <tr>
            <th scope="col" className={head}>
              {t("settings.resume.colFolder")}
            </th>
            <th scope="col" className={cn(head, "w-16 text-center")}>
              {t("settings.resume.colEnabled")}
            </th>
            <th scope="col" className={cn(head, "w-24 text-right")}>
              {t("settings.resume.colCount")}
            </th>
            <th scope="col" className={cn(head, "w-28 text-right")}>
              {t("settings.resume.colAction")}
            </th>
          </tr>
        </thead>
        <tbody>
          {folders.map((folder) => {
            const count = counts.get(folder.folderId) ?? 0
            const name = folderDisplayName(folder)
            return (
              <tr key={folder.folderId} className="border-b border-border text-xs last:border-b-0">
                <td className="px-2 py-1.5 align-middle">
                  <div className="flex min-w-0 items-center gap-1.5">
                    <span className={cn("min-w-0 truncate text-sm font-medium", folder.hidden && "text-muted-foreground")} title={folder.path}>
                      {name}
                    </span>
                    {folder.hidden && <EyeOff role="img" aria-label={t("settings.folders.hiddenBadge")} className="size-3.5 shrink-0 text-muted-foreground" />}
                  </div>
                  <PathLine folder={folder} />
                </td>
                <td className="px-2 py-1.5 text-center align-middle">
                  <Switch
                    aria-label={t("settings.resume.folderEnabled", { name })}
                    checked={enabled.get(folder.folderId) ?? true}
                    disabled={switchesDisabled}
                    onCheckedChange={(next) => onToggle(folder, next)}
                  />
                </td>
                <td className="px-2 py-1.5 text-right align-middle tabular-nums">{formatNumber(count)}</td>
                <td className="px-2 py-1.5 text-right align-middle">
                  <Button size="sm" variant="outline" disabled={count === 0} aria-label={t("settings.resume.resetFolder", { name })} onClick={() => onReset(folder, count)}>
                    {t("settings.resume.reset")}
                  </Button>
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
