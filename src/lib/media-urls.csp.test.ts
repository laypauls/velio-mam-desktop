// 분류: Happy(csp/devCsp 허용) / Validation(스킴 누락 정책 거부)
// 회귀: ImageViewer 가 mammedia 를 <img> 로 쓰는데 img-src 에 없어 설치본에서 이미지 미리보기가 CSP 로 차단됨
import { describe, expect, it } from "vitest"
import conf from "../../src-tauri/tauri.conf.json"
import { MEDIA_SCHEME, THUMB_SCHEME } from "./media-urls"

function parseCsp(policy: string): Map<string, string[]> {
  const out = new Map<string, string[]>()
  for (const part of policy.split(";")) {
    const [name, ...values] = part.trim().split(/\s+/)
    if (name) out.set(name, values)
  }
  return out
}

/** directive 가 `http://<scheme>.localhost` 와 `<scheme>:` 를 모두 허용하는지. */
function allowsScheme(policy: string, directive: string, scheme: string): boolean {
  const values = parseCsp(policy).get(directive) ?? []
  return values.includes(`http://${scheme}.localhost`) && values.includes(`${scheme}:`)
}

const security = conf.app.security as { csp: string; devCsp: string }
const policies: Array<[string, string]> = [
  ["csp", security.csp],
  ["devCsp", security.devCsp],
]

describe("tauri.conf.json CSP 의 커스텀 스킴 허용", () => {
  it.each(policies)("T-ui-csp-01 %s: img-src 가 썸네일 스킴을 허용한다", (_n, policy) => {
    expect(allowsScheme(policy, "img-src", THUMB_SCHEME)).toBe(true)
  })

  it.each(policies)("T-ui-csp-02 %s: img-src 가 원본 미디어 스킴을 허용한다 (이미지 뷰어)", (_n, policy) => {
    expect(allowsScheme(policy, "img-src", MEDIA_SCHEME)).toBe(true)
  })

  it.each(policies)("T-ui-csp-03 %s: media-src 가 원본 미디어 스킴을 허용한다 (영상 뷰어)", (_n, policy) => {
    expect(allowsScheme(policy, "media-src", MEDIA_SCHEME)).toBe(true)
  })

  it("T-ui-csp-04 스킴이 빠졌거나 한쪽 형태만 있는 정책은 거부한다", () => {
    const missing = "default-src 'self'; img-src 'self' data: http://mamthumb.localhost mamthumb:"
    const partial = "img-src http://mammedia.localhost"
    expect(allowsScheme(missing, "img-src", MEDIA_SCHEME)).toBe(false)
    expect(allowsScheme(partial, "img-src", MEDIA_SCHEME)).toBe(false)
    expect(allowsScheme(missing, "media-src", MEDIA_SCHEME)).toBe(false)
  })
})
