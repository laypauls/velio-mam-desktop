import type { PendingVolume } from "@/lib/ipc-contract"

const SEP = "\\"

/** 비교용 정규화 — 대소문자 무시, 슬래시를 백슬래시로 통일, 끝의 구분자 제거 (Windows 경로) */
function norm(p: string): string {
  return p.trim().replace(/\//g, SEP).replace(/\\+$/, "").toLowerCase()
}

/** `path` 가 마운트 지점 자신이거나 그 아래인가 (대소문자 무시) */
export function isInsideMount(path: string, mountPoint: string): boolean {
  const p = norm(path)
  const m = norm(mountPoint)
  return p === m || p.startsWith(m + SEP)
}

/** 드라이브 문자 경로의 마운트 지점(`E:\`) — UNC 등 드라이브 문자가 없으면 null */
export function driveMountOf(path: string): string | null {
  const m = /^([a-z]):[\\/]/i.exec(path.trim() + SEP)
  return m ? `${m[1].toUpperCase()}:${SEP}` : null
}

/** 카드 제목 — 이름이 있으면 `이름 (마운트 지점)`, 없으면 마운트 지점만 */
export function volumeTitle(v: Pick<PendingVolume, "label" | "mountPoint">): string {
  return v.label ? `${v.label} (${v.mountPoint})` : v.mountPoint
}
