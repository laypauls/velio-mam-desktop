// Categories: Happy / Boundary (length, depth) / Validation (parse errors) / Business rule (fail-closed, short-circuit)
// Rust `mam-plugin-manifest` 와 공용 시험 벡터로 결과 일치를 검증한다.
import { describe, expect, it } from "vitest"
import vectors from "../../crates/mam-plugin-manifest/vectors/when.json"
import { evalWhen, parseWhen, WhenError, type WhenContext } from "./when"

interface Case {
  id: string
  expr: string
  context: WhenContext
  expect: boolean | { error: string }
}
const CASES = vectors.cases as Case[]

describe("when evaluator — shared vectors", () => {
  it("T-ui-pmenu-01 [Validation] the shared vector file is loaded (not an empty glob)", () => {
    expect(CASES.length).toBeGreaterThanOrEqual(69)
    expect(Object.keys(vectors.identifiers).sort()).toEqual(["asset.kind", "asset.offline", "selection.count"])
  })

  it.each(CASES.map((c) => [c.id, c] as const))("T-ui-pmenu-02 [Happy] vector %s matches the Rust result", (_id, c) => {
    // Arrange / Act / Assert
    if (typeof c.expect === "boolean") {
      expect(() => parseWhen(c.expr)).not.toThrow()
      expect(evalWhen(c.expr, c.context)).toBe(c.expect) // 실행 오류(컨텍스트 타입)는 false — Rust `eval`
    } else {
      expect(() => parseWhen(c.expr)).toThrow(WhenError)
      try {
        parseWhen(c.expr)
      } catch (e) {
        expect((e as WhenError).kind).toBe(c.expect.error)
      }
    }
  })
})

describe("when evaluator — menu semantics", () => {
  it("T-ui-pmenu-03 [Business rule] evalWhen is fail-closed: parse errors and wrong context types hide the item", () => {
    expect(evalWhen("asset.kind = 'x'", {})).toBe(false)
    expect(evalWhen("asset.offline", { "asset.offline": "yes" })).toBe(false)
  })

  it("T-ui-pmenu-04 [Boundary] a null condition is always shown", () => {
    expect(evalWhen(null, {})).toBe(true)
  })

  it("T-ui-pmenu-05 [Boundary] length is measured in UTF-8 bytes (Rust parity)", () => {
    // '한' = 3 bytes: 'a'=='' + padding that is 256 bytes vs 257 bytes
    const base = "'' == ''"
    const pad = (n: number) => `${base}${" ".repeat(n)}`
    expect(parseWhen(pad(256 - base.length))).toBeTruthy()
    expect(() => parseWhen(pad(257 - base.length))).toThrow(WhenError)
    const korean = `'${"한".repeat(84)}' == ''` // 84*3 + 6 = 258 bytes but 92 chars
    expect(() => parseWhen(korean)).toThrow(WhenError)
  })
})
