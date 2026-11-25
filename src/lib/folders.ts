import type { ManagedFolder } from "@/lib/ipc-contract"

/** 관리 폴더 표시 이름 — 별칭이 있으면 별칭, 없으면 경로. */
export function folderLabel(f: Pick<ManagedFolder, "alias" | "path">): string {
  return f.alias ?? f.path
}

const TRAILING_SEP = /[\\/]+$/
const ROOT_PATH = /^(?:[A-Za-z]:|[\\/]{2}[^\\/]+[\\/][^\\/]+)$/

/** 경로를 (앞부분, 마지막 구분자+마지막 폴더명) 으로 나눈다 — 루트·구분자 없는 경로는 tail 이 빈 문자열. */
export function splitPathTail(path: string): { head: string; tail: string } {
  const trimmed = path.replace(TRAILING_SEP, "")
  const idx = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"))
  if (trimmed === "" || ROOT_PATH.test(trimmed) || idx <= 0 || trimmed[idx - 1] === "\\" || trimmed[idx - 1] === "/") {
    return { head: path, tail: "" }
  }
  return { head: trimmed.slice(0, idx), tail: trimmed.slice(idx) }
}

/** 관리 폴더 표의 이름 — 별칭, 없으면 마지막 폴더명(루트·UNC 공유 루트는 전체 경로). `folderLabel` 과 별개. */
export function folderDisplayName(f: Pick<ManagedFolder, "alias" | "path">): string {
  if (f.alias !== null) return f.alias
  const { tail } = splitPathTail(f.path)
  return tail === "" ? f.path : tail.slice(1)
}
