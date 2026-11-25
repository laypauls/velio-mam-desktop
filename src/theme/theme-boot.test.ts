import { readFileSync } from "node:fs"
import { resolve } from "node:path"
import { describe, expect, it } from "vitest"
import { isDarkTheme, resolveTheme } from "./resolve"

// 케이스 분류: Happy(5x2x2 일치, gray 는 .dark 없음) / Boundary(캐시 없음·잘못됨) / External(localStorage 예외)
const src = readFileSync(resolve(__dirname, "../../public/theme-boot.js"), "utf8").replace(/\r\n/g, "\n")

function runBoot(opts: { stored: string | null | "throw"; osDark: boolean; matchMedia?: boolean }) {
  const classes = new Set<string>()
  const attrs: Record<string, string> = {}
  const win = {
    localStorage: {
      getItem: () => {
        if (opts.stored === "throw") throw new Error("denied")
        return opts.stored
      },
    },
    matchMedia: opts.matchMedia === false ? undefined : () => ({ matches: opts.osDark }),
    document: {
      documentElement: {
        setAttribute: (k: string, v: string) => (attrs[k] = v),
        classList: { add: (c: string) => classes.add(c), remove: (c: string) => classes.delete(c) },
      },
    },
  }
  new Function("window", src)(win)
  return { theme: attrs["data-theme"], dark: classes.has("dark") }
}

describe("theme-boot.js", () => {
  for (const mode of ["system", "light", "gray", "dim", "dark"] as const)
    for (const systemDark of ["dim", "dark"] as const)
      for (const osDark of [false, true])
        it(`T-ui-theme-17 resolveTheme 와 일치 ${mode}/${systemDark}/os=${osDark}`, () => {
          const r = runBoot({ stored: JSON.stringify({ mode, systemDark }), osDark })
          const want = resolveTheme({ mode, systemDark }, osDark)
          expect(r.theme).toBe(want)
          expect(r.dark).toBe(isDarkTheme(want))
        })

  it("T-ui-theme-18 캐시 없음/깨짐/잘못된 값은 기본값(시스템/딤)", () => {
    for (const stored of [null, "{oops", JSON.stringify({ mode: "x", systemDark: "dim" }), JSON.stringify({ mode: "dark" }), "null"]) {
      expect(runBoot({ stored, osDark: true }).theme).toBe("dim")
      expect(runBoot({ stored, osDark: false }).theme).toBe("light")
    }
  })

  it("T-ui-theme-19 localStorage 예외·matchMedia 없음에도 던지지 않는다", () => {
    expect(runBoot({ stored: "throw", osDark: true }).theme).toBe("dim")
    expect(runBoot({ stored: JSON.stringify({ mode: "system", systemDark: "dark" }), osDark: true, matchMedia: false }).theme).toBe("light")
  })
})
