// 분류: Happy / Boundary / Business rule
// 배포 범위(Rust normal 의존성·번들된 npm 패키지)와 THIRD_PARTY_NOTICES.md 표 ↔ licenses/ 원문 대응.
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { afterAll, beforeAll, describe, expect, it } from "vitest"
import { collectNpmPackages, cssImportPackages, packageFromModuleId } from "./npm.mjs"
import { hasPinnedText, licenseIds, parseNoticeTables } from "./notices.mjs"
import { selectRustPackages } from "./rust.mjs"

const LICENSES_DIR = resolve(process.cwd(), "licenses")
const norm = (s: string) => s.replace(/\\/g, "/")

let tmp: string
beforeAll(() => {
  tmp = mkdtempSync(join(tmpdir(), "lic-scope-"))
})
afterAll(() => {
  rmSync(tmp, { recursive: true, force: true })
})

describe("distribution scope - Rust", () => {
  const pkg = (name: string, procMacro = false) => ({ id: name, name, version: "1.0.0", targets: [{ kind: [procMacro ? "proc-macro" : "lib"] }] })
  const dep = (to: string, ...kinds: (string | null)[]) => ({ pkg: to, dep_kinds: kinds.map((kind) => ({ kind, target: null })) })
  const meta = {
    workspace_members: ["app"],
    packages: [pkg("app"), pkg("normal-dep"), pkg("build-only"), pkg("build-only-child"), pkg("dev-only"), pkg("both"), pkg("derive", true), pkg("derive-helper"), pkg("transitive")],
    resolve: {
      nodes: [
        { id: "app", deps: [dep("normal-dep", null), dep("build-only", "build"), dep("dev-only", "dev"), dep("both", null, "build"), dep("derive", null)] },
        { id: "normal-dep", deps: [dep("transitive", null)] },
        { id: "build-only", deps: [dep("build-only-child", null)] },
        { id: "derive", deps: [dep("derive-helper", null)] },
        { id: "both", deps: [] },
      ],
    },
  }

  it("T-lic-28 [Business rule] build 전용·dev 전용·proc-macro(와 그 전용 의존성)는 제외하고 normal+build 양쪽이면 유지한다", () => {
    // Act
    const names = selectRustPackages(meta).map((p: { name: string }) => p.name)
    // Assert
    expect(names).toEqual(["both", "normal-dep", "transitive"])
  })

  it("T-lic-29 [Boundary] 워크스페이스 멤버만 있으면 빈 목록", () => {
    expect(selectRustPackages({ workspace_members: ["app"], packages: [pkg("app")], resolve: { nodes: [{ id: "app", deps: [] }] } })).toEqual([])
  })
})

describe("distribution scope - npm", () => {
  it("T-lic-30 [Happy] 모듈 id 에서 패키지를 찾는다 (pnpm 중첩·scope·쿼리·역슬래시)", () => {
    expect(packageFromModuleId("C:/r/node_modules/.pnpm/react@19/node_modules/react/index.js")).toEqual({ name: "react", dir: "C:/r/node_modules/.pnpm/react@19/node_modules/react" })
    expect(packageFromModuleId("C:\\r\\node_modules\\@scope\\pkg\\dist\\a.js?v=1")).toEqual({ name: "@scope/pkg", dir: "C:/r/node_modules/@scope/pkg" })
    expect(packageFromModuleId("\0vite/preload-helper.js")).toBeNull()
    expect(packageFromModuleId("C:/r/src/main.tsx")).toBeNull()
  })

  it("T-lic-31 [Business rule] CSS @import 한 패키지만 추출하고 상대 경로·URL 은 무시한다", () => {
    const css = [
      '@import "tailwindcss" source(".");',
      '@import "tw-animate-css";',
      '@import "shadcn/tailwind.css";',
      '@import "@font/inter";',
      '@import "./local.css";',
      '@import url("https://x.example/a.css");',
      '@import "/abs.css";',
    ].join("\n")
    expect(cssImportPackages(css)).toEqual(["@font/inter", "shadcn", "tailwindcss", "tw-animate-css"])
  })

  it("T-lic-32 [Business rule] 번들에 들어간 패키지와 CSS/글꼴로만 들어간 패키지는 포함하고, 번들에 없는 패키지는 제외한다", () => {
    // Arrange
    const root = join(tmp, "npm-root")
    const mk = (rel: string, pj: object) => {
      mkdirSync(join(root, rel), { recursive: true })
      writeFileSync(join(root, rel, "package.json"), JSON.stringify(pj))
      return norm(join(root, rel))
    }
    mkdirSync(root, { recursive: true })
    writeFileSync(join(root, "package.json"), JSON.stringify({ name: "the-app", version: "0.0.0" }))
    const bundled = mk("node_modules/.pnpm/lib-a@1.2.3/node_modules/lib-a", { name: "lib-a", version: "1.2.3", license: "MIT", author: "A <a@x.y>" })
    mk("node_modules/css-only", { name: "css-only", version: "2.0.0", license: { type: "ISC" } })
    mk("node_modules/@font/face", { name: "@font/face", version: "3.0.0", licenses: [{ type: "OFL-1.1" }] })
    mk("node_modules/dev-tool", { name: "dev-tool", version: "9.9.9", license: "MIT" })
    // Act
    const pkgs = collectNpmPackages({ root, moduleIds: [`${bundled}/dist/index.js?x`, `${norm(root)}/src/main.tsx`], cssImports: ["css-only", "@font/face", "missing-pkg"] })
    // Assert
    expect(pkgs.map((p: { name: string }) => p.name)).toEqual(["@font/face", "css-only", "lib-a"])
    expect(pkgs.map((p: { license: string }) => p.license)).toEqual(["OFL-1.1", "ISC", "MIT"])
    expect(pkgs.find((p: { name: string }) => p.name === "lib-a")).toMatchObject({ version: "1.2.3", kind: "npm", authors: ["A"] })
  })
})

describe("THIRD_PARTY_NOTICES.md bundled-library tables vs licenses/", () => {
  const notices = () => readFileSync(resolve(process.cwd(), "THIRD_PARTY_NOTICES.md"), "utf8")

  it("T-lic-33 [Happy] 라이선스 열의 SPDX id·예외·특허 라이선스를 모두 뽑는다", () => {
    expect(licenseIds("BSD-3-Clause WITH PCRE2-exception")).toEqual(["BSD-3-Clause", "PCRE2-exception"])
    expect(licenseIds("FTL (chosen from FTL OR GPL-2.0-or-later)")).toEqual(["FTL", "GPL-2.0"])
    expect(licenseIds("BSD-2-Clause (+ Alliance for Open Media Patent License 1.0)")).toEqual(["AOM-Patent-1.0", "BSD-2-Clause"])
    expect(licenseIds("LGPL-2.0-or-later; `src/packet-830.c`: **GPL-2.0-only** (see its `COPYING.md`)")).toEqual(["GPL-2.0", "LGPL-2.0"])
    expect(licenseIds("MIT (Fontconfig license, see `COPYING`)")).toEqual(["MIT"])
    expect(licenseIds('MIT-Modern-Variant ("Old MIT")')).toEqual(["MIT", "MIT-Modern-Variant"])
  })

  it("T-lic-34 [Business rule] 표(1.1.1, 1.2, 1.3.1)에 나오는 모든 라이선스 id 는 licenses/ 에 원문이 있다", () => {
    // Arrange / Act
    const rows = parseNoticeTables(notices())
    const missing = rows.flatMap((r: { license: string; component: string }) =>
      licenseIds(r.license)
        .filter((id: string) => !hasPinnedText(LICENSES_DIR, id))
        .map((id: string) => `${r.component}: ${id}`),
    )
    // Assert
    expect(rows.length).toBeGreaterThan(80)
    expect(new Set(rows.map((r: { section: string }) => r.section))).toEqual(new Set(["1.1.1", "1.2", "1.3.1"]))
    expect(missing).toEqual([])
  })

  it("T-lic-35 [Business rule] 표의 모든 라이선스가 생성 파일에 절로 들어 있다", () => {
    const gen = readFileSync(resolve(process.cwd(), "THIRD_PARTY_LICENSES.md"), "utf8").replace(/\r\n/g, "\n")
    const ids = new Set(parseNoticeTables(notices()).flatMap((r: { license: string }) => licenseIds(r.license)))
    for (const id of ids) expect(gen, id).toMatch(new RegExp(`^## .*\\(${id.replace(/[.+]/g, "\\$&")}[,)]`, "m"))
  })

  it("T-lic-36 [Validation] 알 수 없는 id 는 원문 없음으로 보고된다", () => {
    expect(hasPinnedText(LICENSES_DIR, "Not-A-License-9.9")).toBe(false)
    expect(licenseIds("Not-A-License-9.9")).toEqual(["Not-A-License-9.9"])
  })
})
