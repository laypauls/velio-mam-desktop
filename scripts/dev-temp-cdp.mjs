// `pnpm dev:temp` 의 WebView2 원격 디버깅(CDP) 포트 결정·인자 조합 순수 함수.
export const DEFAULT_CDP_PORT = 19333
const FLAG = "--remote-debugging-port"

/** MAM_DEV_CDP_PORT 값 → 포트 번호, 0 이면 null(끔). 미지정·빈 문자열은 기본값. 잘못된 값은 Error. */
export function resolveCdpPort(raw) {
  if (raw === undefined || raw === null || String(raw).trim() === "") return DEFAULT_CDP_PORT
  const text = String(raw).trim()
  if (!/^\d+$/.test(text)) throw new Error(`MAM_DEV_CDP_PORT must be an integer 0-65535 (got "${text}")`)
  const port = Number(text)
  if (port === 0) return null
  if (port > 65535) throw new Error(`MAM_DEV_CDP_PORT must be in 1..65535 (got ${port})`)
  return port
}

/** 기존 값에 이미 --remote-debugging-port 가 있는가. */
export const hasCdpFlag = (existing) => typeof existing === "string" && existing.includes(FLAG)

/** 기존 인자 뒤에 공백으로 덧붙인다. 이미 있으면 그대로 반환. */
export function mergeBrowserArgs(existing, port) {
  if (hasCdpFlag(existing)) return existing
  const base = (existing ?? "").trim()
  const ours = `${FLAG}=${port}`
  return base ? `${base} ${ours}` : ours
}
