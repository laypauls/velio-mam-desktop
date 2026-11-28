// 분류: Happy / Boundary / External
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { afterEach, describe, expect, it } from "vitest"
import { ensureDevSidecars, resolveTargetDir } from "./dev-sidecars.mjs"

const dirs: string[] = []
function makeRoot(withSources: boolean) {
  const root = mkdtempSync(join(tmpdir(), "dev-sidecars-"))
  dirs.push(root)
  if (withSources) {
    const bin = join(root, "src-tauri", "binaries")
    mkdirSync(join(bin, "dlls"), { recursive: true })
    writeFileSync(join(bin, "whisper-cli-x86_64-pc-windows-msvc.exe"), "exe")
    for (const n of ["whisper.dll", "ggml.dll", "ggml-cpu-x64.dll", "avcodec-62.dll"]) writeFileSync(join(bin, "dlls", n), n)
  }
  return root
}
afterEach(() => {
  while (dirs.length) rmSync(dirs.pop()!, { recursive: true, force: true })
})

describe("dev-sidecars ensureDevSidecars", () => {
  it("T-devside-01 [Happy] 없으면 whisper-cli.exe 와 whisper/ggml DLL 만 target/debug 에 복사", () => {
    const root = makeRoot(true)
    const copied = ensureDevSidecars({ root, env: {}, warn: () => {} })
    const out = join(root, "target", "debug")
    expect(readdirSync(out).sort()).toEqual(["ggml-cpu-x64.dll", "ggml.dll", "whisper-cli.exe", "whisper.dll"])
    expect(copied).toHaveLength(4)
    expect(readFileSync(join(out, "whisper-cli.exe"), "utf8")).toBe("exe")
  })
  it("T-devside-02 [Boundary] 같은 크기·최신이면 다시 복사하지 않음, 크기가 다르면 갱신", () => {
    const root = makeRoot(true)
    ensureDevSidecars({ root, env: {}, warn: () => {} })
    expect(ensureDevSidecars({ root, env: {}, warn: () => {} })).toEqual([])
    writeFileSync(join(root, "src-tauri", "binaries", "dlls", "ggml.dll"), "ggml.dll-longer")
    expect(ensureDevSidecars({ root, env: {}, warn: () => {} })).toHaveLength(1)
  })
  it("T-devside-03 [External] 원본 없음이면 던지지 않고 fetch-sidecars 경고", () => {
    const root = makeRoot(false)
    const warns: string[] = []
    expect(ensureDevSidecars({ root, env: {}, warn: (m: string) => warns.push(m) })).toEqual([])
    expect(warns.join("\n")).toMatch(/fetch-sidecars\.ps1/)
  })
  it("T-devside-04 [Boundary] CARGO_TARGET_DIR 를 따름", () => {
    const root = makeRoot(true)
    expect(resolveTargetDir(root, { CARGO_TARGET_DIR: "out" })).toBe(join(root, "out"))
    ensureDevSidecars({ root, env: { CARGO_TARGET_DIR: "out" }, warn: () => {} })
    expect(readdirSync(join(root, "out", "debug"))).toContain("whisper-cli.exe")
  })
})
