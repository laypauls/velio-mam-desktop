// Categories: Validation (T-ui-i18n-01, T-ui-i18n-02) — 번역 키 누락 정적 검사
import { describe, expect, it } from "vitest"
import ko from "./locales/ko.json"

/** 소스 원문(`?raw`) — 테스트·shadcn 생성물(`components/ui/`)·테스트 헬퍼는 검사 대상에서 제외한다. */
const RAW = import.meta.glob<string>(
  ["/src/**/*.{ts,tsx}", "!/src/**/*.test.{ts,tsx}", "!/src/components/ui/**", "!/src/test/**"],
  { query: "?raw", import: "default", eager: true },
)

/** `\r\n` → `\n` 정규화 (도구가 새로 만든 파일은 CRLF 일 수 있다). */
const FILES: Record<string, string> = Object.fromEntries(
  Object.entries(RAW).map(([path, text]) => [path.replace(/^\/src\//, ""), text.replace(/\r\n/g, "\n")]),
)

function read(rel: string): string {
  const text = FILES[rel]
  if (text === undefined) throw new Error(`source not scanned: ${rel}`)
  return text
}

function hasKey(key: string): boolean {
  let node: unknown = ko
  for (const part of key.split(".")) {
    if (typeof node !== "object" || node === null || !(part in node)) return false
    node = (node as Record<string, unknown>)[part]
  }
  return typeof node === "string"
}

/** `t("a.b")`, `t('a.b')`, `i18nKey="a.b"` — 리터럴만. 템플릿 리터럴(`${`)·변수는 아래 동적 키 군에서 검증한다. */
const LITERAL_RE = /(?:\bt\(\s*|\bi18nKey=)(?:"([^"\n]+)"|'([^'\n]+)')/g

describe("i18n key guard", () => {
  it("T-ui-i18n-01 [Validation] every literal key used via t()/i18nKey exists in ko.json", () => {
    // Arrange
    const missing: string[] = []
    let found = 0
    // Act
    for (const [file, text] of Object.entries(FILES)) {
      for (const m of text.matchAll(LITERAL_RE)) {
        const key = m[1] ?? m[2]
        found++
        if (!hasKey(key)) missing.push(`${file}: ${key}`)
      }
    }
    // Assert (스캐너가 아무것도 못 찾는 회귀도 잡는다)
    expect(found).toBeGreaterThan(50)
    expect(missing).toEqual([])
  })

  it("T-ui-i18n-02 [Validation] dynamic key families exist: errors.<ErrorCode>, update.error.<code>", () => {
    // Arrange: ErrorCode 유니온을 contract 소스에서 읽는다 (`errors.<code>` — lib/errors.ts errorKey)
    const contract = read("lib/ipc-contract.ts")
    const union = /export type ErrorCode =([\s\S]*?)\n\n/.exec(contract)?.[1] ?? ""
    const aiUnion = /export type AiErrorCode =([\s\S]*?)\n\nexport const AI_LIMITS/.exec(read("lib/ipc-contract-ai.ts"))?.[1] ?? ""
    const codes = [
      ...[...union.matchAll(/"([a-z_]+)"/g)].map((m) => m[1]),
      ...[...aiUnion.matchAll(/^\s*\|\s*"([a-z_]+)"/gm)].map((m) => m[1]),
    ]
    const updateUnion = /export interface UpdateError \{\s*code:([\s\S]*?)message/.exec(read("lib/ipc.ts"))?.[1] ?? ""
    const updateCodes = [...updateUnion.matchAll(/"([a-z_]+)"/g)].map((m) => m[1])

    // Act
    const missing = [
      ...codes.map((c) => `errors.${c}`),
      ...updateCodes.map((c) => `update.error.${c}`),
      "update.error.unknown",
    ].filter((k) => !hasKey(k))

    // Assert
    expect(codes.length).toBeGreaterThanOrEqual(12)
    expect(updateCodes.length).toBeGreaterThanOrEqual(5)
    expect(missing).toEqual([])
  })
})
