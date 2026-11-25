import { ChevronDown, ChevronRight, Folder, TriangleAlert } from "lucide-react"
import { useState } from "react"
import { useTranslation } from "react-i18next"
import { useSubfolders } from "@/hooks/useLibrary"
import { folderLabel } from "@/lib/folders"
import { formatNumber } from "@/lib/format"
import type { ManagedFolder } from "@/lib/ipc-contract"
import { cn } from "@/lib/utils"
import { driveMountOf } from "@/lib/volumes"
import { useSearchStore } from "@/stores"
import { FolderEjectMenu, hasEjectMenu } from "./FolderEjectMenu"

interface RowProps {
  folderId: number
  relPath: string | null
  name: string
  /** 루트 행에서 별칭 사용 시 전체 경로를 툴팁으로 노출 */
  title?: string
  hasChildren: boolean
  assetCount: number
  depth: number
  offline?: boolean
  /** 루트 행만 — 꺼내기 준비 상태면 오프라인 표시에 "꺼낼 준비됨" 툴팁을 단다 */
  ejectPrepared?: boolean
  /** 루트 행만 — 우클릭 꺼내기 메뉴의 대상 (같은 디스크 폴더 수 포함) */
  eject?: { folder: ManagedFolder; sameDiskCount: number }
}

function FolderRow({ folderId, relPath, name, title, hasChildren, assetCount, depth, offline, ejectPrepared, eject }: RowProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const view = useSearchStore((s) => s.view)
  const setView = useSearchStore((s) => s.setView)
  const active = view.kind === "folder" && view.folderId === folderId && view.relPath === relPath
  const Chevron = expanded ? ChevronDown : ChevronRight

  const row = (
      <div
        className={cn("flex items-center gap-1 rounded-md pr-2 text-sm hover:bg-muted", active && "bg-accent text-accent-foreground")}
        // 트리 깊이별 들여쓰기는 동적 값이라 style 이 필요하다
        style={{ paddingLeft: depth * 12 }}
      >
        {hasChildren ? (
          <button
            type="button"
            aria-label={t(expanded ? "folders.collapse" : "folders.expand", { name })}
            aria-expanded={expanded}
            onClick={() => setExpanded((v) => !v)}
            className="flex size-6 shrink-0 items-center justify-center rounded-sm"
          >
            <Chevron aria-hidden="true" className="size-4" />
          </button>
        ) : (
          <span className="size-6 shrink-0" />
        )}
        <button
          type="button"
          aria-current={active ? "true" : undefined}
          title={title}
          onClick={() => setView({ kind: "folder", folderId, relPath })}
          className="flex min-w-0 flex-1 items-center gap-1.5 py-1 text-left"
        >
          <Folder aria-hidden="true" className="size-4 shrink-0 text-muted-foreground" />
          <span className="truncate">{name}</span>
        </button>
        {offline && (
          <span title={ejectPrepared ? t("folders.eject.preparedBadge") : undefined} className="flex shrink-0">
            <TriangleAlert
              role="img"
              aria-label={t(ejectPrepared ? "folders.eject.preparedBadge" : "asset.availability.offline")}
              className="size-4 text-muted-foreground"
            />
          </span>
        )}
        <span className="text-xs text-muted-foreground tabular-nums">{formatNumber(assetCount)}</span>
      </div>
  )

  return (
    <li>
      {eject ? <FolderEjectMenu folder={eject.folder} sameDiskCount={eject.sameDiskCount}>{row}</FolderEjectMenu> : row}
      {expanded && <SubfolderList folderId={folderId} parentRel={relPath} depth={depth + 1} />}
    </li>
  )
}

/** 하위 폴더는 펼칠 때만 조회한다 (지연 로딩). */
function SubfolderList({ folderId, parentRel, depth }: { folderId: number; parentRel: string | null; depth: number }) {
  const { t } = useTranslation()
  const { data, isPending, isError } = useSubfolders(folderId, parentRel, true)
  if (isPending) return <p className="px-3 py-1 text-xs text-muted-foreground">{t("common.loading")}</p>
  if (isError) return <p role="alert" className="px-3 py-1 text-xs text-destructive">{t("folders.subfolderError")}</p>
  return (
    <ul>
      {data.map((n) => (
        <FolderRow key={n.relPath} folderId={folderId} relPath={n.relPath} name={n.name} hasChildren={n.hasChildren} assetCount={n.assetCount} depth={depth} />
      ))}
    </ul>
  )
}

/** `allFolders` — 숨긴 폴더까지 포함한 전체 목록 (같은 디스크 폴더 수 계산용, 기본은 `folders`) */
export function FolderTree({ folders, allFolders = folders }: { folders: ManagedFolder[]; allFolders?: ManagedFolder[] }) {
  const diskCount = (f: ManagedFolder) => {
    const mount = driveMountOf(f.path)
    return mount === null ? 1 : allFolders.filter((o) => driveMountOf(o.path) === mount).length
  }
  return (
    <ul>
      {folders.map((f) => (
        <FolderRow
          key={f.folderId}
          folderId={f.folderId}
          relPath={null}
          name={folderLabel(f)}
          title={f.alias !== null ? f.path : undefined}
          hasChildren
          assetCount={f.assetCount}
          depth={0}
          offline={f.availability === "offline"}
          ejectPrepared={f.ejectPrepared}
          eject={hasEjectMenu(f) ? { folder: f, sameDiskCount: diskCount(f) } : undefined}
        />
      ))}
    </ul>
  )
}
