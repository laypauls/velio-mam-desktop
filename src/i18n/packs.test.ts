// Categories: Validation (T-ui-i18n-03/04/05) — 번들 언어팩(src-tauri/locales/*.json) 이 기준 ko.json 과 구조·placeholder·내용이 일치하는지 검사.
// diffPack 순수 헬퍼 + 인라인 fixture 로 실패 경로를 실제 파일과 무관하게 검증한다 (T-ui-i18n-03f/04f/05f).
import { describe, expect, it } from "vitest"
import ko from "./locales/ko.json"

const EXPECTED_PACKS = ["de", "en", "es", "fr", "ja"]

type Json = Record<string, unknown>

/** `/src-tauri/locales/en.json` → `en`. 실제 파일이 아직 없으면 빈 객체. */
const RAW = import.meta.glob<Json>("../../src-tauri/locales/*.json", { import: "default", eager: true })
const PACKS: Record<string, Json> = Object.fromEntries(
  Object.entries(RAW).map(([path, json]) => [/([^/]+)\.json$/.exec(path)?.[1] ?? path, json]),
)

/** 최상위 `_meta` 를 제외한 leaf 경로 → 값. 배열·null 도 leaf 로 취급해 타입 검사에서 걸러낸다. */
function flatten(node: Json, prefix = "", top = true): Map<string, unknown> {
  const out = new Map<string, unknown>()
  for (const [k, v] of Object.entries(node)) {
    if (top && k === "_meta") continue
    const key = prefix ? `${prefix}.${k}` : k
    if (typeof v === "object" && v !== null && !Array.isArray(v)) {
      for (const [ck, cv] of flatten(v as Json, key, false)) out.set(ck, cv)
    } else out.set(key, v)
  }
  return out
}

function placeholders(s: string): string[] {
  return [...new Set([...s.matchAll(/\{\{\s*([^}\s,]+)[^}]*\}\}/g)].map((m) => m[1]))].sort()
}

const HANGUL = /[가-힣]/

export interface PackDiff {
  missing: string[]
  extra: string[]
  placeholderMismatch: string[]
  invalid: string[]
  hangul: string[]
}

/** 기준(ko) 대비 pack 의 차이를 키 목록으로 돌려준다. 각 목록이 비어 있어야 통과. */
export function diffPack(ref: Json, pack: Json): PackDiff {
  const r = flatten(ref)
  const p = flatten(pack)
  const diff: PackDiff = { missing: [], extra: [], placeholderMismatch: [], invalid: [], hangul: [] }
  for (const k of r.keys()) if (!p.has(k)) diff.missing.push(k)
  for (const k of p.keys()) if (!r.has(k)) diff.extra.push(k)
  for (const [k, v] of p) {
    if (typeof v !== "string" || v.trim() === "") {
      diff.invalid.push(k)
      continue
    }
    if (HANGUL.test(v)) diff.hangul.push(k)
    const rv = r.get(k)
    if (typeof rv === "string") {
      const want = placeholders(rv).join("|")
      const got = placeholders(v).join("|")
      if (want !== got) diff.placeholderMismatch.push(`${k}: expected [${want}] got [${got}]`)
    }
  }
  const meta = (pack as { _meta?: { name?: unknown } })._meta?.name
  if (typeof meta !== "string" || meta.trim() === "") diff.invalid.push("_meta.name")
  return diff
}

const REF = ko as Json

describe("bundled language packs (real files)", () => {
  it("T-ui-i18n-03 [Validation] pack set is exactly de/en/es/fr/ja and key sets equal ko.json", () => {
    // Arrange
    const names = Object.keys(PACKS).sort()
    // Act
    const report: Record<string, { missing: string[]; extra: string[] }> = {}
    for (const [name, pack] of Object.entries(PACKS)) {
      const d = diffPack(REF, pack)
      if (d.missing.length || d.extra.length) report[name] = { missing: d.missing, extra: d.extra }
    }
    // Assert
    expect(names).toEqual(EXPECTED_PACKS)
    expect(report).toEqual({})
  })

  it("T-ui-i18n-04 [Validation] {{placeholder}} sets equal ko.json for every key", () => {
    // Arrange / Act
    const report: Record<string, string[]> = {}
    for (const [name, pack] of Object.entries(PACKS)) {
      const d = diffPack(REF, pack)
      if (d.placeholderMismatch.length) report[name] = d.placeholderMismatch
    }
    // Assert (팩이 하나도 로드되지 않는 회귀도 잡는다)
    expect(Object.keys(PACKS).length).toBeGreaterThan(0)
    expect(report).toEqual({})
  })

  it("T-ui-i18n-05 [Validation] leaves are non-empty strings, _meta.name set, no Hangul remains", () => {
    // Arrange / Act
    const report: Record<string, { invalid: string[]; hangul: string[] }> = {}
    for (const [name, pack] of Object.entries(PACKS)) {
      const d = diffPack(REF, pack)
      if (d.invalid.length || d.hangul.length) report[name] = { invalid: d.invalid, hangul: d.hangul }
    }
    // Assert
    expect(Object.keys(PACKS).length).toBeGreaterThan(0)
    expect(report).toEqual({})
  })
})

describe("diffPack guard (inline fixtures)", () => {
  const ref = {
    a: { b: "Hello {{name}}", c: "Plain" },
    d: "{{x}} and {{ y }}",
  }
  const okPack = { _meta: { name: "English" }, a: { b: "Hi {{name}}", c: "Simple" }, d: "{{y}} / {{x}}" }

  it("T-ui-i18n-03f [Validation] identical structure passes; missing and extra keys are reported", () => {
    // Arrange
    const bad = { _meta: { name: "X" }, a: { b: "Hi {{name}}", z: "extra" }, d: "{{x}}{{y}}" }
    // Act
    const ok = diffPack(ref, okPack)
    const d = diffPack(ref, bad)
    // Assert
    expect(ok).toEqual({ missing: [], extra: [], placeholderMismatch: [], invalid: [], hangul: [] })
    expect(d.missing).toEqual(["a.c"])
    expect(d.extra).toEqual(["a.z"])
  })

  it("T-ui-i18n-04f [Validation] wrong, missing or extra placeholder is reported with the key", () => {
    // Arrange
    const bad = { _meta: { name: "X" }, a: { b: "Hi {{nome}}", c: "Plain {{oops}}" }, d: "{{x}} only" }
    // Act
    const d = diffPack(ref, bad)
    // Assert
    expect(d.placeholderMismatch).toHaveLength(3)
    expect(d.placeholderMismatch[0]).toContain("a.b")
    expect(d.placeholderMismatch[1]).toContain("a.c")
    expect(d.placeholderMismatch[2]).toContain("d")
  })

  it("T-ui-i18n-05f [Validation] empty/non-string leaves, blank _meta.name and Hangul are reported", () => {
    // Arrange
    const bad = { _meta: { name: " " }, a: { b: "", c: ["x"] }, d: "남은 {{x}} {{y}}" }
    // Act
    const d = diffPack(ref, bad)
    // Assert
    expect(d.invalid.sort()).toEqual(["_meta.name", "a.b", "a.c"])
    expect(d.hangul).toEqual(["d"])
  })
})
