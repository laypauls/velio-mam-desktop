// 분류: Happy / Boundary / Validation / Business rule
// scripts/licenses/*.mjs (THIRD_PARTY_LICENSES.md 생성 로직) — 순수 함수 + 임시 픽스처 디렉터리. cargo/pnpm 은 호출하지 않는다.
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { afterAll, beforeAll, describe, expect, it } from "vitest"
import { resolvePackage } from "./collect.mjs"
import { diffSummary, groupKey, parseGenerated, renderDocument } from "./render.mjs"
import { evaluateSpdx, parseSpdx } from "./spdx.mjs"
import { detectSpdx, extractCopyrights, findLicenseFiles, isCopyrightLine, stripCopyrights } from "./texts.mjs"

const LICENSES_DIR = resolve(process.cwd(), "licenses")

const MIT_BODY = `Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM.`
const mit = (copyright?: string) => `${copyright ? `${copyright}\n\n` : ""}${MIT_BODY}\n`
const APACHE = `                                 Apache License\n                           Version 2.0, January 2004\n\n   TERMS AND CONDITIONS\n\n   Copyright [yyyy] [name of copyright owner]\n`

let tmp: string
let n = 0
/** files: name -> content. Returns the package directory. */
function pkgDir(files: Record<string, string>): string {
  const dir = join(tmp, `pkg${n++}`)
  mkdirSync(dir, { recursive: true })
  for (const [name, text] of Object.entries(files)) writeFileSync(join(dir, name), text)
  return dir
}
const pkg = (name: string, files: Record<string, string>, license: string | null, extra: object = {}) => ({
  name,
  version: "1.0.0",
  kind: "rust" as const,
  dir: pkgDir(files),
  license,
  authors: [] as string[],
  ...extra,
})
const resolveOne = (p: ReturnType<typeof pkg>, opts: object = {}) => resolvePackage(p, { licensesDir: LICENSES_DIR, exceptions: {}, overrides: {}, ...opts })

beforeAll(() => {
  tmp = mkdtempSync(join(tmpdir(), "lic-test-"))
})
afterAll(() => {
  rmSync(tmp, { recursive: true, force: true })
})

describe("license file discovery", () => {
  it("T-lic-01 [Happy] LICENSE-MIT / LICENSE-APACHE / COPYING / NOTICE / LICENCE.md 를 대소문자 무관하게 찾고 CRLF 를 LF 로 바꾼다", () => {
    // Arrange
    const dir = pkgDir({
      "LICENSE-MIT": "mit\r\ntext",
      "license-apache": "apache",
      COPYING: "copying",
      "NOTICE.txt": "notice",
      "LICENCE.md": "licence",
      "README.md": "readme",
      "license.rs": "fn main() {}",
      "licenses.json": "{}",
    })
    mkdirSync(join(dir, "LICENSES"))
    // Act
    const files = findLicenseFiles(dir)
    // Assert
    expect(files.map((f) => f.name)).toEqual(["COPYING", "LICENCE.md", "LICENSE-MIT", "NOTICE.txt", "license-apache"])
    expect(files.find((f) => f.name === "LICENSE-MIT")?.text).toBe("mit\ntext")
    expect(files.find((f) => f.name === "NOTICE.txt")?.kind).toBe("notice")
    expect(files.find((f) => f.name === "COPYING")?.kind).toBe("license")
  })

  it("T-lic-02 [Boundary] 없는 디렉터리·빈 디렉터리는 빈 목록", () => {
    expect(findLicenseFiles(join(tmp, "does-not-exist"))).toEqual([])
    expect(findLicenseFiles(pkgDir({}))).toEqual([])
  })

  it("T-lic-03 [Business rule] 라이선스 문구를 SPDX id 로 식별하고 알 수 없으면 빈 배열", () => {
    expect(detectSpdx(mit("Copyright (c) 2020 A"))).toEqual(["MIT"])
    expect(detectSpdx(APACHE)).toEqual(["Apache-2.0"])
    expect(detectSpdx("some custom terms")).toEqual([])
  })
})

describe("copyright lines", () => {
  it("T-lic-04 [Happy] 실제 저작권 줄만 추출하고 템플릿·경고문 줄은 제외한다", () => {
    // Arrange
    const text = `Copyright (c) 2015 Foo Bar\n(c) 2020 Baz\nCopyright Julian\nCopyright [yyyy] [name of copyright owner]\nCOPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM\nCopyright notice and this permission`
    // Act / Assert
    expect(extractCopyrights([text])).toEqual(["Copyright (c) 2015 Foo Bar", "(c) 2020 Baz", "Copyright Julian"])
    expect(isCopyrightLine("COPYRIGHT HOLDERS BE LIABLE")).toBe(false)
    expect(stripCopyrights(mit("Copyright (c) 2015 Foo"))).toBe(MIT_BODY)
  })
})

describe("SPDX allowlist", () => {
  it("T-lic-05 [Happy] OR 는 하나라도 허용이면 통과하고 MIT 를 우선 고른다", () => {
    expect(evaluateSpdx("Apache-2.0 OR MIT")).toMatchObject({ ok: true, picks: ["MIT"] })
    expect(evaluateSpdx("MIT/Apache-2.0")).toMatchObject({ ok: true, picks: ["MIT"] })
    expect(evaluateSpdx("GPL-3.0-only OR ISC")).toMatchObject({ ok: true, picks: ["ISC"] })
  })

  it("T-lic-06 [Business rule] AND 는 모두 허용이어야 하고 전부 고른다", () => {
    expect(evaluateSpdx("(MIT OR Apache-2.0) AND Unicode-3.0")).toMatchObject({ ok: true, picks: ["MIT", "Unicode-3.0"] })
    const bad = evaluateSpdx("MIT AND GPL-3.0-only")
    expect(bad.ok).toBe(false)
    expect(bad.violations).toEqual(["GPL-3.0-only"])
  })

  it("T-lic-07 [Business rule] WITH 는 허용된 예외(LLVM-exception)만 통과한다", () => {
    expect(evaluateSpdx("Apache-2.0 WITH LLVM-exception")).toMatchObject({ ok: true, picks: ["Apache-2.0"] })
    expect(evaluateSpdx("Apache-2.0 WITH Classpath-exception-2.0").ok).toBe(false)
  })

  it("T-lic-08 [Validation] 허용 밖 라이선스·잘못된 식은 실패한다 (GPL/AGPL/미지정)", () => {
    for (const e of ["GPL-3.0-or-later", "AGPL-3.0", "SSPL-1.0", "LicenseRef-custom", "MIT OR", "(MIT", "", "MIT MIT"]) {
      expect(evaluateSpdx(e).ok, e).toBe(false)
    }
    expect(() => parseSpdx("MIT AND")).toThrow()
  })

  it("T-lic-09 [Boundary] 소문자 id·연산자도 정식 표기로 돌려준다", () => {
    expect(evaluateSpdx("mit or apache-2.0")).toMatchObject({ ok: true, picks: ["MIT"] })
  })
})

describe("resolvePackage", () => {
  it("T-lic-10 [Happy] MIT OR Apache-2.0 이고 두 파일이 있으면 MIT 전문을 쓰고 저작권 줄은 본문에서 분리한다", () => {
    // Arrange
    const p = pkg("dual", { "LICENSE-MIT": mit("Copyright (c) 2019 Dual Author"), "LICENSE-APACHE": APACHE }, "MIT OR Apache-2.0")
    // Act
    const r = resolveOne(p)
    // Assert
    expect(r.problems).toEqual([])
    expect(r.entries).toHaveLength(1)
    expect(r.entries[0]).toMatchObject({ id: "MIT", source: "file", file: "LICENSE-MIT", text: MIT_BODY })
    expect(r.copyrights).toEqual(["Copyright (c) 2019 Dual Author"])
  })

  it("T-lic-11 [Business rule] 라이선스 파일이 없으면 SPDX 표준 전문으로 채우고 저작권은 authors 에서 가져온다", () => {
    // Arrange
    const p = pkg("nofile", {}, "MIT", { authors: ["Jane Doe <jane@example.com>"] })
    // Act
    const r = resolveOne(p)
    // Assert
    expect(r.problems).toEqual([])
    expect(r.entries[0]).toMatchObject({ id: "MIT", source: "spdx" })
    expect(r.entries[0].text).toContain("Permission is hereby granted, free of charge")
    expect(r.entries[0].text).not.toMatch(/<year>|<copyright holders>/)
    expect(r.copyrights).toEqual(["Copyright holders: see authors: Jane Doe <jane@example.com>"])
  })

  it("T-lic-12 [Business rule] MIT OR Apache-2.0 인데 Apache 파일만 있으면 MIT 는 SPDX 표준 전문을 쓴다", () => {
    const r = resolveOne(pkg("apache-only", { "LICENSE-APACHE": APACHE }, "MIT OR Apache-2.0", { authors: ["A"] }))
    expect(r.entries).toHaveLength(1)
    expect(r.entries[0]).toMatchObject({ id: "MIT", source: "spdx" })
  })

  it("T-lic-13 [Validation] 라이선스 미기재(UNKNOWN)는 실패 목록에 오른다", () => {
    for (const lic of [null, "", "UNKNOWN"]) {
      const r = resolveOne(pkg("mystery", { LICENSE: mit("Copyright (c) 2020 X") }, lic))
      expect(r.problems.map((x) => x.type), String(lic)).toEqual(["unknown"])
    }
  })

  it("T-lic-14 [Validation] 허용 목록 밖(GPL)은 violation, 예외 목록(allow)에 있으면 통과한다", () => {
    // Arrange
    const gpl = pkg("gpl-crate", { LICENSE: "GPL text" }, "GPL-3.0-only")
    const blue = pkg("blue", { LICENSE: "Blue Oak Model License\nVersion 1.0.0\nCopyright 2020 Foo" }, "BlueOak-1.0.0")
    // Act
    const rGpl = resolveOne(gpl, { exceptions: { "gpl-crate": { allow: [], reason: "x" } } })
    const rBlueDenied = resolveOne(blue)
    const rBlueOk = resolveOne(blue, { exceptions: { blue: { allow: ["BlueOak-1.0.0"], reason: "permissive" } } })
    // Assert
    expect(rGpl.problems[0]).toMatchObject({ type: "violation" })
    expect(rBlueDenied.problems[0]).toMatchObject({ type: "violation" })
    expect(rBlueOk.problems).toEqual([])
    expect(rBlueOk.entries[0]).toMatchObject({ id: "BlueOak-1.0.0", source: "file" })
  })

  it("T-lic-15 [Business rule] 저작권을 어디서도 못 찾으면 failure 로 보고하고, 확인된 override 가 있으면 사용한다", () => {
    const p = () => pkg("anon", { LICENSE: mit() }, "MIT")
    expect(resolveOne(p()).problems.map((x) => x.type)).toEqual(["copyright"])
    const r = resolveOne(p(), { overrides: { anon: ["Copyright (c) 2021 Checked Holder"] } })
    expect(r.problems).toEqual([])
    expect(r.copyrights).toEqual(["Copyright (c) 2021 Checked Holder"])
  })

  it("T-lic-16 [Business rule] NOTICE 파일은 패키지에 붙고 저작권 줄이 없으면 NOTICE 에서 가져온다", () => {
    const r = resolveOne(pkg("noticed", { LICENSE: APACHE, NOTICE: "Noticed Product\nCopyright 2018 The Noticed Authors\n" }, "Apache-2.0"))
    expect(r.problems).toEqual([])
    expect(r.notices).toEqual([{ file: "NOTICE", text: "Noticed Product\nCopyright 2018 The Noticed Authors\n" }])
    expect(r.copyrights).toEqual(["Copyright 2018 The Noticed Authors"])
  })
})

describe("grouping and rendering", () => {
  const make = (name: string, text: string, copyright: string, extra: object = {}) => ({
    name,
    version: "1.0.0",
    kind: "rust" as const,
    entries: [{ id: "MIT", text: stripCopyrights(text), source: "file" as const }],
    copyrights: [copyright],
    notices: [],
    ...extra,
  })
  const counts = { rust: 3, npm: 0 }
  const render = (packages: ReturnType<typeof make>[]) => renderDocument({ packages, cargoSha: "a".repeat(64), pnpmSha: "b".repeat(64), counts })

  it("T-lic-17 [Happy] 같은 전문은 한 절로 묶고 패키지별 저작권은 각 항목에 남긴다", () => {
    // Arrange
    const pkgs = [make("b-crate", mit("Copyright (c) 2020 B"), "Copyright (c) 2020 B"), make("a-crate", mit("Copyright (c) 2018 A"), "Copyright (c) 2018 A")]
    // Act
    const md = render(pkgs)
    // Assert
    expect(md.match(/^## /gm)).toHaveLength(1)
    expect(md).toContain("## MIT License (MIT) — 2 packages")
    expect(md.indexOf("`a-crate`")).toBeLessThan(md.indexOf("`b-crate`"))
    expect(md).toContain("- `a-crate` 1.0.0 (rust) — Copyright (c) 2018 A")
    expect(md).toContain("- `b-crate` 1.0.0 (rust) — Copyright (c) 2020 B")
    expect(md.match(/Permission is hereby granted/g)).toHaveLength(1)
  })

  it("T-lic-18 [Boundary] 구두점·제목 줄·줄바꿈만 다른 전문은 같은 절이고, 문구가 다르면 variant 절로 나눈다", () => {
    // Arrange
    const reflowed = `MIT License\n\n${MIT_BODY.replace(/\n/g, " ").replace("free of charge,", "free of charge;")}`
    const different = mit().replace("without restriction", "with restrictions")
    expect(groupKey(reflowed)).toBe(groupKey(MIT_BODY))
    const pkgs = [make("a", mit(), "Copyright (c) 1 A"), make("b", reflowed, "Copyright (c) 2 B"), make("c", different, "Copyright (c) 3 C")]
    // Act
    const md = render(pkgs)
    // Assert
    expect(md).toContain("## MIT License (MIT) — 2 packages")
    expect(md).toContain("## MIT License (MIT, variant 2) — 1 packages")
  })

  it("T-lic-19 [Business rule] 입력 순서와 무관하게 출력이 동일하다 (재현 가능한 출력)", () => {
    // Arrange
    const pkgs = [
      make("z", mit(), "Copyright (c) 1 Z"),
      make("a", mit(), "Copyright (c) 1 A"),
      make("m", mit().replace("without restriction", "with restrictions"), "Copyright (c) 1 M"),
      make("a", mit(), "Copyright (c) 1 A", { version: "0.9.0" }),
    ]
    // Act
    const forward = render(pkgs)
    const backward = render([...pkgs].reverse())
    // Assert
    expect(backward).toBe(forward)
    expect(forward.indexOf("`a` 0.9.0")).toBeLessThan(forward.indexOf("`a` 1.0.0"))
  })

  it("T-lic-20 [Business rule] NOTICE 는 해당 패키지 소제목 아래에만 나오고, 전문에 ``` 가 있으면 더 긴 울타리를 쓴다", () => {
    // Arrange
    const noticed = make("noticed", mit(), "Copyright (c) 1 N", { notices: [{ file: "NOTICE", text: "Product\n```\ncode\n```\n" }] })
    const plain = make("plain", mit(), "Copyright (c) 1 P")
    // Act
    const md = render([noticed, plain])
    // Assert
    expect(md.match(/^### NOTICE$/gm)).toHaveLength(1)
    expect(md).toContain("#### `noticed` 1.0.0")
    expect(md).not.toContain("#### `plain`")
    expect(md).toContain("````\nProduct\n```\ncode\n```\n````")
  })

  it("T-lic-21 [Business rule] 머리말에 잠금 파일 SHA-256 과 패키지 수가 기록되고 번들 항목은 수에서 제외된다", () => {
    // Arrange
    const bundled = make("FFmpeg", mit(), "Copyright (c) FFmpeg", { kind: "bundled" })
    // Act
    const md = render([make("a", mit(), "Copyright (c) A"), bundled])
    // Assert
    expect(md.split("\n")[1]).toBe(`<!-- generated-by: scripts/dump-licenses.mjs; cargo-lock-sha256: ${"a".repeat(64)}; pnpm-lock-sha256: ${"b".repeat(64)}; packages: 1 -->`)
    expect(md).toContain("- `FFmpeg` 1.0.0 (bundled) — Copyright (c) FFmpeg")
  })

  it("T-lic-22 [Business rule] 저작권 줄의 마크다운 특수문자는 이스케이프되어 HTML 로 해석되지 않는다", () => {
    const md = render([make("esc", mit(), "Copyright (c) 2020 A <a@b.c> *x*")])
    expect(md).toContain("Copyright (c) 2020 A \\<a@b.c\\> \\*x\\*")
  })
})

describe("--check diff", () => {
  const base = (md: string) => parseGenerated(md)
  const doc = (cargo: string, pnpm: string, bullets: string[]) =>
    `# Third-Party Licenses\n<!-- generated-by: scripts/dump-licenses.mjs; cargo-lock-sha256: ${cargo}; pnpm-lock-sha256: ${pnpm}; packages: ${bullets.length} -->\n\n${bullets.join("\n")}\n`

  it("T-lic-23 [Happy] 머리말과 패키지 목록을 되읽는다 (CRLF 포함)", () => {
    // Arrange
    const md = doc("abc123", "def456", ["- `serde` 1.0.0 (rust) — Copyright (c) A", "- `@scope/pkg` 2.0.0 (npm) — x"])
    // Act
    const parsed = base(md.replace(/\n/g, "\r\n"))
    // Assert
    expect(parsed).toMatchObject({ cargoSha: "abc123", pnpmSha: "def456", count: 2 })
    expect([...parsed!.packages].sort()).toEqual(["npm:@scope/pkg@2.0.0", "rust:serde@1.0.0"])
  })

  it("T-lic-24 [Business rule] 잠금 파일 해시·패키지 증감을 요약하고 같으면 Up to date", () => {
    // Arrange
    const parsed = base(doc("aa00", "bb00", ["- `a` 1.0.0 (rust) — x", "- `b` 1.0.0 (npm) — y"]))!
    // Act
    const same = diffSummary(parsed, { cargoSha: "aa00", pnpmSha: "bb00", packages: new Set(parsed.packages) })
    const changed = diffSummary(parsed, { cargoSha: "ff", pnpmSha: "bb00", packages: new Set(["rust:a@1.0.0", "rust:c@2.0.0"]) })
    // Assert
    expect(same.at(-1)).toBe("Up to date.")
    expect(changed).toContain("Cargo.lock sha256: CHANGED")
    expect(changed).toContain("pnpm-lock.yaml sha256: unchanged")
    expect(changed).toContain("  + rust:c@2.0.0")
    expect(changed).toContain("  - npm:b@1.0.0")
    expect(changed.at(-1)).toMatch(/Regeneration may be needed/)
  })

  it("T-lic-25 [Validation] 머리말이 없는 파일은 비교하지 않고 안내만 한다", () => {
    expect(base("# nothing")).toBeNull()
    expect(diffSummary(null, { cargoSha: "a", pnpmSha: "b", packages: new Set() })[0]).toMatch(/no readable generated-by header/)
  })
})

describe("committed THIRD_PARTY_LICENSES.md format contract", () => {
  const md = readFileSync(resolve(process.cwd(), "THIRD_PARTY_LICENSES.md"), "utf8").replace(/\r\n/g, "\n")

  it("T-lic-26 [Happy] 머리말이 있고, 울타리 밖 `## ` 절은 모두 '— N packages' 이며 절마다 Packages·License text 를 가진다", () => {
    // Arrange
    const sections: string[] = []
    let fenced = ""
    let current = ""
    for (const line of md.split("\n")) {
      const f = /^(`{3,})/.exec(line)
      if (f && !fenced) fenced = f[1]
      else if (f && fenced && line.trim() === fenced) fenced = ""
      if (!fenced && !f && /^## /.test(line)) {
        if (current) sections.push(current)
        current = line + "\n"
      } else if (current) current += line + "\n"
    }
    if (current) sections.push(current)
    // Act / Assert
    expect(parseGenerated(md)).not.toBeNull()
    expect(sections.length).toBeGreaterThan(10)
    for (const s of sections) {
      expect(s.split("\n")[0], s.slice(0, 80)).toMatch(/^## .+ \(.+\) — \d+ packages$|^## .+ — \d+ packages$/)
      expect(s).toContain("### Packages\n")
      expect(s).toContain("### License text\n")
    }
  })

  it("T-lic-27 [Business rule] 번들 프로그램 전문(LGPL-2.1, LGPL-3.0, GPL-3.0, MPL-2.0, OFL-1.1, CDLA)과 onnxruntime 고지가 들어 있다", () => {
    for (const h of ["(LGPL-2.1)", "(LGPL-3.0)", "(GPL-3.0)", "(MPL-2.0)", "(OFL-1.1)", "(CDLA-Permissive-2.0)", "ONNX Runtime third-party notices"]) {
      expect(md, h).toContain(h)
    }
    expect(md).not.toContain("\r")
  })
})
