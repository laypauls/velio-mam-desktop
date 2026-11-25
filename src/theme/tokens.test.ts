/// <reference types="node" />
import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { describe, expect, it } from "vitest"

// 분류: Business rule (테마별 토큰 누락 방지) / Validation (정적 CSS 검사)
// project.md 함정: 소스를 읽어 정적 검증할 때 \r\n -> \n 정규화
const css = readFileSync(resolve(__dirname, "../index.css"), "utf8").replace(/\r\n/g, "\n")

/** 셀렉터 정확 일치 블록의 본문(첫 `{` ~ 짝 `}`)을 반환한다. */
function block(selector: string): string {
  const start = css.indexOf(`\n${selector} {`)
  if (start < 0) throw new Error(`블록 없음: ${selector}`)
  const open = css.indexOf("{", start)
  const close = css.indexOf("\n}", open)
  return css.slice(open + 1, close)
}

function tokens(body: string): string[] {
  return [...body.matchAll(/^\s*(--[\w-]+)\s*:/gm)].map((m) => m[1])
}

// --radius 는 테마와 무관한 기하 토큰이라 제외
const LIGHT_SELECTOR = ":root,\n.light"
const rootTokens = tokens(block(LIGHT_SELECTOR)).filter((t) => t !== "--radius")

describe("테마 토큰 완전성", () => {
  it("T-ui-theme-tok-01 :root 가 토큰을 정의한다 (검사 대상 비어있지 않음)", () => {
    // Arrange/Act/Assert
    expect(rootTokens).toContain("--background")
    expect(rootTokens.length).toBeGreaterThan(20)
  })

  it("T-ui-theme-tok-02 .dark 가 :root 의 모든 토큰을 정의한다", () => {
    const dark = new Set(tokens(block(".dark")))
    const missing = rootTokens.filter((t) => !dark.has(t))
    expect(missing).toEqual([])
  })

  it('T-ui-theme-tok-03 .dark[data-theme="dim"] 가 :root 의 모든 토큰을 정의한다', () => {
    const dim = new Set(tokens(block('.dark[data-theme="dim"]')))
    const missing = rootTokens.filter((t) => !dim.has(t))
    expect(missing).toEqual([])
  })

  it("T-ui-theme-tok-04 color-scheme 이 라이트/다크에 지정된다", () => {
    expect(block(LIGHT_SELECTOR)).toMatch(/color-scheme:\s*light/)
    expect(block(".dark")).toMatch(/color-scheme:\s*dark/)
  })

  it("T-ui-theme-tok-05 다크 계열 sidebar-primary 는 무채색이다 (chroma 0 또는 중립 hue)", () => {
    for (const sel of [".dark", '.dark[data-theme="dim"]']) {
      const m = block(sel).match(/--sidebar-primary:\s*oklch\(([\d.]+)\s+([\d.]+)/)
      expect(m, sel).not.toBeNull()
      expect(Number(m![2]), sel).toBeLessThan(0.02)
    }
  })

  it('T-ui-theme-tok-07 :root[data-theme="gray"] 가 :root 의 모든 토큰을 정의하고 color-scheme 은 light', () => {
    const body = block(':root[data-theme="gray"],\n.light[data-theme="gray"]')
    const gray = new Set(tokens(body))
    expect(rootTokens.filter((t) => !gray.has(t))).toEqual([])
    expect(body).toMatch(/color-scheme:\s*light/)
  })

  it("T-ui-theme-tok-06블록 검출 실패 시 오류 (실패 경로)", () => {
    expect(() => block(".nonexistent")).toThrow()
  })
})
