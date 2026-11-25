/**
 * 루트 고지 문서를 빌드 시 번들한다(런타임 네트워크 없음).
 * `import.meta.glob` — 파일이 아직 없어도(생성 전) 빌드가 깨지지 않고 빈 문자열이 된다.
 */
const NOTICES = import.meta.glob("../../../THIRD_PARTY_NOTICES.md", { query: "?raw", import: "default", eager: true })
const LICENSES = import.meta.glob("../../../THIRD_PARTY_LICENSES.md", { query: "?raw", import: "default", eager: true })

function first(files: Record<string, unknown>): string {
  const v = Object.values(files)[0]
  return typeof v === "string" ? v : ""
}

export const NOTICES_MD = first(NOTICES)
export const LICENSES_MD = first(LICENSES)
