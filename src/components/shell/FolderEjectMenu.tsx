import type { ReactNode } from "react"
import { useTranslation } from "react-i18next"
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuTrigger } from "@/components/ui/context-menu"
import { useVolumeEject } from "@/hooks/useVolumeEject"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { driveMountOf } from "@/lib/volumes"

/**
 * 백엔드가 `ejectable` 로 표시한 폴더(이동식·외장 디스크)만 꺼내기 메뉴를 가진다 — 내장·시스템·MAMData·네트워크는 메뉴 없음.
 * 이미 오프라인인데 준비 상태도 아닌 폴더는 꺼낼 것이 없으므로 항목이 없다(준비 상태 폴더는 "감시 다시 시작").
 */
export function hasEjectMenu(folder: Pick<ManagedFolder, "ejectable" | "availability" | "ejectPrepared">): boolean {
  return folder.ejectable && (folder.ejectPrepared || folder.availability === "online")
}

interface Props {
  folder: ManagedFolder
  /** 같은 디스크의 관리 폴더 수 (숨김 포함, 자기 자신 포함) */
  sameDiskCount: number
  /** 우클릭 대상 행 — Radix 가 자식에 contextmenu 핸들러를 붙인다 (Shift+F10·메뉴 키 포함) */
  children: ReactNode
}

/** 사이드바 관리 폴더 우클릭 메뉴 — "디스크 꺼내기 준비 (E:)" 또는 준비 상태에서 "감시 다시 시작 (E:)" 한 항목만 둔다. */
export function FolderEjectMenu({ folder, sameDiskCount, children }: Props) {
  const { t } = useTranslation()
  const { prepare, resume } = useVolumeEject()
  if (!hasEjectMenu(folder)) return <>{children}</>
  const drive = driveMountOf(folder.path)?.slice(0, 2) ?? folder.path
  const prepared = folder.ejectPrepared
  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>{children}</ContextMenuTrigger>
      <ContextMenuContent aria-label={t("folders.eject.menuLabel", { drive })}>
        <ContextMenuItem
          className="flex-col items-start gap-0"
          disabled={prepare.isPending || resume.isPending}
          onSelect={() => (prepared ? resume.mutate(folder.folderId) : prepare.mutate(folder.folderId))}
        >
          <span>{t(prepared ? "folders.eject.resume" : "folders.eject.prepare", { drive })}</span>
          {sameDiskCount > 1 && <span className="text-xs break-keep text-muted-foreground">{t("folders.eject.allOnDisk", { count: sameDiskCount })}</span>}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}
