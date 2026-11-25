import { Eye, EyeOff, HardDrive, Image, Info, Music, Network, Settings as SettingsIcon, TriangleAlert, Usb, Video, type LucideIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { AvailabilityBadge } from "@/components/gallery/AvailabilityBadge"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { useSetFolderHidden } from "@/hooks/useFolderAdmin"
import { errorKey } from "@/lib/errors"
import { folderDisplayName, splitPathTail } from "@/lib/folders"
import { formatNumber } from "@/lib/format"
import type { ManagedFolder, MediaType } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { ExperimentalMark, ScanStallLine, WatchHealthBadge } from "./FolderStatusBadges"
import { fallbackReason, isNoAutoReflect, rescanNextText, rescanPolicyText, watchName } from "./detection/summary"

const VOLUME_ICONS: Record<ManagedFolder["volumeKind"], LucideIcon> = { fixed: HardDrive, network: Network, removable: Usb }

const TYPE_ICONS: readonly [MediaType, LucideIcon][] = [
  ["image", Image],
  ["video", Video],
  ["audio", Music],
]

/** 좁은 폭(컨테이너 < 42rem)에서 숨기는 열 — 머리글(`FoldersTab`)과 같은 클래스를 쓴다. */
export const NARROW_HIDDEN = "@max-2xl:hidden"

function TypeIcons({ folder }: { folder: ManagedFolder }) {
  const { t } = useTranslation()
  return (
    <span className="flex items-center gap-1.5">
      {TYPE_ICONS.map(([type, Icon]) => {
        const on = folder.mediaTypes[type]
        const label = t(on ? "settings.folders.table.typeIncluded" : "settings.folders.table.typeExcluded", { type: t(`asset.type.${type}`) })
        return (
          <span key={type} role="img" aria-label={label} title={label} className={cn("relative inline-flex", !on && "text-muted-foreground opacity-50")}>
            <Icon aria-hidden="true" className="size-4" />
            {!on && <span aria-hidden="true" className="absolute top-1/2 -right-0.5 -left-0.5 h-px -rotate-45 bg-current" />}
          </span>
        )
      })}
    </span>
  )
}

/** 볼륨 종류 아이콘 + 경로 한 줄 — 앞부분만 잘리고 마지막 폴더명은 유지된다. */
export function PathLine({ folder }: { folder: ManagedFolder }) {
  const { t } = useTranslation()
  const { head, tail } = splitPathTail(folder.path)
  const VolumeIcon = VOLUME_ICONS[folder.volumeKind]
  const label = t(`settings.folders.volumeIconLabel.${folder.volumeKind}`)
  return (
    <div className="mt-0.5 flex min-w-0 items-center gap-1.5 pl-3.5 text-muted-foreground">
      <VolumeIcon role="img" aria-label={label} className="size-3.5 shrink-0">
        <title>{label}</title>
      </VolumeIcon>
      <span className="flex min-w-0 flex-1 font-mono text-xs" title={folder.path}>
        <span className="min-w-0 truncate">{head}</span>
        {tail !== "" && <span className="max-w-[60%] shrink-0 truncate">{tail}</span>}
      </span>
    </div>
  )
}

interface Props {
  folder: ManagedFolder
  defaultHours: number
  onOpenSettings: () => void
  /** 실험 표시를 붙일 감지기 ID (`list_change_detectors` 의 `experimental`) */
  experimental?: ReadonlySet<string>
}

/** 관리 폴더 표의 한 줄 — 비교·훑어보기용 요약. 상세 편집은 [설정] 다이얼로그에서 한다. */
export function FolderRow({ folder, defaultHours, onOpenSettings, experimental }: Props) {
  const { t } = useTranslation()
  const setHidden = useSetFolderHidden()
  const online = folder.availability === "online"
  const watch = watchName(t, folder)
  const reason = fallbackReason(t, folder)
  const policy = rescanPolicyText(t, folder, defaultHours)
  const next = rescanNextText(t, folder)
  const settingsLabel = t("settings.folders.dialog.openLabel", { path: folder.path })
  const cell = "px-2 py-1.5 align-middle"

  return (
    <tr className="border-b border-border text-xs last:border-b-0">
      <td className={cell}>
        <div className="flex min-w-0 items-center gap-1.5">
          <span
            role="img"
            aria-label={t(online ? "asset.availability.online" : "asset.availability.offline")}
            className={cn("size-2 shrink-0 rounded-full", online ? "bg-primary" : "bg-muted-foreground/60")}
          />
          <span className={cn("min-w-0 truncate text-sm font-medium", folder.hidden && "text-muted-foreground")} title={folder.path}>
            {folderDisplayName(folder)}
          </span>
          {folder.hidden && <EyeOff role="img" aria-label={t("settings.folders.hiddenBadge")} className="size-3.5 shrink-0 text-muted-foreground" />}
          {!online && <AvailabilityBadge availability="offline" />}
          {folder.scanState !== "idle" && (
            <Badge variant={folder.scanState === "failed" ? "destructive" : "outline"} className="shrink-0 rounded-sm">
              {t(`settings.folders.scanState.${folder.scanState}`)}
            </Badge>
          )}
          {isNoAutoReflect(folder) && (
            <TriangleAlert
              role="img"
              aria-label={t("settings.folders.detection.noAutoBadge")}
              className="size-3.5 shrink-0 text-destructive"
            />
          )}
        </div>
        <PathLine folder={folder} />
        {folder.scanStall && <ScanStallLine stall={folder.scanStall} />}
        {setHidden.isError && (
          <p role="alert" className="text-xs text-destructive">
            {t(errorKey(setHidden.error))}
          </p>
        )}
      </td>
      <td className={cell}>
        <TypeIcons folder={folder} />
      </td>
      <td className={cn(cell, NARROW_HIDDEN)}>
        <div className="flex min-w-0 items-center gap-1">
          {watch === null ? (
            <span className="truncate text-muted-foreground">{t("watch.notWatching")}</span>
          ) : (
            <span className="truncate" title={watch}>
              {watch}
            </span>
          )}
          {folder.watchActive !== null && experimental?.has(folder.watchActive) && <ExperimentalMark />}
          {reason !== null && <Info role="img" aria-label={reason} className="size-3.5 shrink-0 text-muted-foreground" />}
          <WatchHealthBadge folder={folder} />
        </div>
      </td>
      <td className={cell}>
        <div className="flex min-w-0 flex-col">
          <span className="truncate" title={policy}>
            {policy}
          </span>
          {next !== null && (
            <span className="truncate text-xs tabular-nums text-muted-foreground" title={next}>
              {next}
            </span>
          )}
        </div>
      </td>
      <td className={cn(cell, NARROW_HIDDEN, "text-right tabular-nums")}>{formatNumber(folder.assetCount)}</td>
      <td className={cell}>
        <div className="flex items-center justify-end gap-0.5">
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t(folder.hidden ? "settings.folders.unhide" : "settings.folders.hide", { path: folder.path })}
            disabled={setHidden.isPending}
            onClick={() => setHidden.mutate({ folderId: folder.folderId, hidden: !folder.hidden })}
          >
            {folder.hidden ? <EyeOff aria-hidden="true" /> : <Eye aria-hidden="true" />}
          </Button>
          <Button size="icon-sm" variant="ghost" aria-label={settingsLabel} title={settingsLabel} onClick={onOpenSettings}>
            <SettingsIcon aria-hidden="true" />
          </Button>
        </div>
      </td>
    </tr>
  )
}
