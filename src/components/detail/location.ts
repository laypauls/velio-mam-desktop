import { folderLabel } from "@/lib/folders"
import type { ManagedFolder } from "@/lib/ipc-contract"

const SEP = /[\\/]+/

/**
 * 위치 한 줄 — 별칭이 있으면 `별칭 › 하위 경로`(파일명 제외, 폴더 루트 기준), 없으면 전체 경로.
 * 파일이 폴더 루트 밖이거나 경로를 알 수 없으면 전체 경로로 되돌린다.
 */
export function locationText(path: string, folder: Pick<ManagedFolder, "alias" | "path"> | null): string {
  if (folder === null || folder.alias === null) return path
  const root = folder.path.split(SEP).filter(Boolean)
  const dir = path.split(SEP).filter(Boolean).slice(0, -1)
  const inside = root.length <= dir.length && root.every((seg, i) => seg.toLowerCase() === dir[i].toLowerCase())
  if (!inside) return path
  const rel = dir.slice(root.length)
  return rel.length === 0 ? folderLabel(folder) : `${folderLabel(folder)} › ${rel.join("\\")}`
}
