// 분류: Happy / Validation / Business rule / External(해시 불일치·다운로드)
// fetch-sidecars.ps1 의 libmpv 구성. 함수(Get-Verified, Expand-LibMpv, Assert-LgplLibMpv)는 스크립트에서 소스를 잘라
// Windows PowerShell 5.1 로 실제 실행한다(임시 폴더·가짜 zip/DLL 만 사용 — 네트워크는 로컬 HTTP 서버뿐).
import { execFile } from "node:child_process"
import { createHash } from "node:crypto"
import { mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs"
import { createServer, type Server } from "node:http"
import type { AddressInfo } from "node:net"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { promisify } from "node:util"
import { afterAll, beforeAll, describe, expect, it } from "vitest"

const run = promisify(execFile)
const read = (rel: string) => readFileSync(resolve(process.cwd(), rel), "utf8").replace(/\r\n/g, "\n")
const script = read("scripts/fetch-sidecars.ps1")
const overlay = JSON.parse(read("src-tauri/tauri.sidecars.conf.json"))
const baseConf = JSON.parse(read("src-tauri/tauri.conf.json"))

/** 최상위 `function Name ... \n}` 블록의 소스. 없으면 오류. */
function fnSource(name: string): string {
  const m = new RegExp(`^function ${name}\\b[\\s\\S]*?^}`, "m").exec(script)
  if (!m) throw new Error(`function ${name} not found`)
  return m[0]
}

const sha256 = (b: Buffer | string) => createHash("sha256").update(b).digest("hex")
const sandbox: string[] = []
function tmp(): string {
  const d = mkdtempSync(join(tmpdir(), "mam-sidecar-test-"))
  sandbox.push(d)
  return d
}
afterAll(() => sandbox.forEach((d) => rmSync(d, { recursive: true, force: true })))

/** `$ErrorActionPreference=Stop` 로 body 를 실행. 성공이면 stdout, 실패면 `{ error: <메시지>, out }`. */
async function ps(functions: string[], body: string): Promise<{ out: string; error: string | null }> {
  const dir = tmp()
  const file = join(dir, "t.ps1")
  const text = [
    "$ErrorActionPreference = 'Stop'",
    "$ProgressPreference = 'SilentlyContinue'",
    ...functions,
    "try {",
    body,
    "} catch { Write-Output ('ERR:' + $_.Exception.Message) }",
  ].join("\n")
  writeFileSync(file, text, "utf8")
  const { stdout } = await run("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", file], { timeout: 90_000 })
  const err = /^ERR:(.*)$/m.exec(stdout)
  return { out: stdout, error: err ? err[1].trim() : null }
}

const psq = (p: string) => `'${p.replace(/'/g, "''")}'`

/** 파일 트리를 zip 으로 묶는다(Compress-Archive 가 하위 폴더를 유지한다). */
async function makeZip(files: Record<string, Buffer | string>): Promise<string> {
  const src = tmp()
  for (const [rel, data] of Object.entries(files)) {
    const p = join(src, rel)
    mkdirSync(join(p, ".."), { recursive: true })
    writeFileSync(p, data)
  }
  const zip = join(tmp(), "x.zip")
  const r = await ps([], `Compress-Archive -Path ${psq(join(src, "*"))} -DestinationPath ${psq(zip)}`)
  expect(r.error).toBeNull()
  return zip
}

/** 1MB 이상의 가짜 PE(MZ 머리말) — 문자열을 DLL 안에 심는다. */
function fakeDll(...strings: string[]): Buffer {
  return Buffer.concat([Buffer.from("MZ"), Buffer.alloc(1_100_000, 0), ...strings.map((s) => Buffer.from(`\0${s}\0`, "latin1"))])
}
const LGPL = "libavcodec license: LGPL version 3 or later"

describe("fetch-sidecars.ps1 / tauri.sidecars.conf.json 정적 구성", () => {
  it("T-sidecar-10 [Business rule] libVLC 허용 목록·플러그인 가드가 없고 libmpv 해시가 캐시 키($Want)에 들어 있다", () => {
    // Assert
    expect(script).not.toMatch(/\$Vlc(Plugins|RequiredPlugins|Url|Sha)/)
    expect(script).not.toMatch(/vlc-COPYING|vlc-AUTHORS/)
    expect(script).toMatch(/\$MpvSha = '[0-9a-f]{64}'/)
    expect(script).toMatch(/\$Want = "[^"]*\$MpvSha[^"]*"/)
    expect(script).toContain("-not (Test-Path (Join-Path $Bin 'libvlc'))") // 예전 레이아웃이 남아 있으면 다시 설치
  })

  it("T-sidecar-11 [Business rule] libmpv 는 공식 v0.42.0 lgpl 자산 + 고정 SHA-256", () => {
    expect(script).toContain("https://github.com/mpv-player/mpv/releases/download/v0.42.0/libmpv-v0.42.0-x86_64-w64-mingw32-lgpl.zip")
    expect(script).toContain("8e5f77430efc53ef175804ba6618527e3af0448a0dcabf04d3cc72659d1a6fce")
    expect(script).not.toMatch(/mingw32-gpl|\/latest\//)
  })

  it("T-sidecar-12 [Business rule] 오버레이: 워커 externalBin + libmpv 리소스(libvlc 없음) + 워커 빌드 beforeBuildCommand", () => {
    // Assert
    expect(overlay.bundle.externalBin).toEqual(expect.arrayContaining(["binaries/mam-player-worker", "binaries/ffmpeg", "binaries/ffprobe", "binaries/whisper-cli"]))
    expect(overlay.bundle.resources["binaries/libmpv"]).toBe("libmpv")
    expect(JSON.stringify(overlay)).not.toMatch(/libvlc/i)
    const before: string = overlay.build.beforeBuildCommand
    expect(before.startsWith(baseConf.build.beforeBuildCommand)).toBe(true) // 프런트 빌드는 그대로 먼저
    expect(before).toContain("scripts/build-player-worker.ps1")
  })

  it("T-sidecar-13 [Business rule] 기본 tauri.conf.json 에는 externalBin·libmpv 가 없다(없으면 모든 cargo 빌드가 실패)", () => {
    expect(baseConf.bundle.externalBin).toBeUndefined()
    expect(JSON.stringify(baseConf.bundle.resources)).not.toMatch(/libmpv|libvlc/)
  })

  it("T-sidecar-14 [Business rule] 워커 스크립트는 --locked 릴리스 빌드 후 target-triple 접미사 이름으로 binaries 에 둔다", () => {
    // Arrange
    const w = read("scripts/build-player-worker.ps1")
    // Assert
    expect(w).toMatch(/cargo build --release --locked -p mam-player-worker/)
    expect(w).toContain('"mam-player-worker-$Triple.exe"')
    expect(w).toContain("$LASTEXITCODE")
  })

  it("T-sidecar-15 [Business rule] 플러그인 워커도 externalBin + --locked 릴리스 빌드 스크립트 + beforeBuildCommand, 파일명은 런타임 탐색(locate.rs)과 일치", () => {
    // Arrange
    const w = read("scripts/build-plugin-worker.ps1")
    const locate = read("crates/mam-plugin-runtime/src/runtime/locate.rs")
    // Assert
    expect(overlay.bundle.externalBin).toContain("binaries/mam-plugin-worker")
    expect(overlay.build.beforeBuildCommand).toContain("scripts/build-plugin-worker.ps1")
    expect(w).toMatch(/cargo build --release --locked -p mam-plugin-worker/)
    expect(w).toContain('"mam-plugin-worker-$Triple.exe"')
    expect(w).toContain("$LASTEXITCODE")
    expect(locate).toContain('WORKER_EXE: &str = "mam-plugin-worker.exe"')
  })
})

describe.skipIf(process.platform !== "win32")("fetch-sidecars.ps1 함수 실행 (Windows PowerShell)", () => {
  let server: Server
  let url = ""
  const served = Buffer.from("served-but-wrong")
  beforeAll(async () => {
    server = createServer((_req, res) => res.end(served))
    await new Promise<void>((ok) => server.listen(0, "127.0.0.1", ok))
    url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/libmpv.zip`
  })
  afterAll(() => new Promise<void>((ok) => server.close(() => ok())))

  const names = `$DllName = 'libmpv-2.dll'; $LicName = 'LICENSE.LGPL'`

  it("T-sidecar-15 [Happy] Expand-LibMpv 는 DLL 과 LICENSE.LGPL 두 파일만 꺼낸다(include·jar 등은 무시)", async () => {
    // Arrange
    const dll = fakeDll(LGPL)
    const zip = await makeZip({ "libmpv-2.dll": dll, "LICENSE.LGPL": "lgpl text", "libmpv.dll.a": "x", "include/mpv/client.h": "h", "libbluray-j2se-1.5.1.jar": "j" })
    const dest = join(tmp(), "libmpv")
    // Act
    const r = await ps([fnSource("Expand-LibMpv")], `${names}; Expand-LibMpv ${psq(zip)} ${psq(dest)} $DllName $LicName`)
    // Assert
    expect(r.error).toBeNull()
    expect(readdirSync(dest).sort()).toEqual(["LICENSE.LGPL", "libmpv-2.dll"])
    expect(sha256(readFileSync(join(dest, "libmpv-2.dll")))).toBe(sha256(dll))
  })

  it("T-sidecar-16 [Validation] 압축에 libmpv-2.dll 이 없으면 중단한다", async () => {
    const zip = await makeZip({ "LICENSE.LGPL": "lgpl text", "libmpv.dll.a": "x" })
    const r = await ps([fnSource("Expand-LibMpv")], `${names}; Expand-LibMpv ${psq(zip)} ${psq(join(tmp(), "d"))} $DllName $LicName`)
    expect(r.error).toBe("libmpv-2.dll not found in libmpv archive")
  })

  it("T-sidecar-17 [Validation] 압축에 LICENSE.LGPL 이 없으면 중단한다", async () => {
    const zip = await makeZip({ "libmpv-2.dll": fakeDll(LGPL) })
    const r = await ps([fnSource("Expand-LibMpv")], `${names}; Expand-LibMpv ${psq(zip)} ${psq(join(tmp(), "d"))} $DllName $LicName`)
    expect(r.error).toBe("LICENSE.LGPL not found in libmpv archive")
  })

  it("T-sidecar-18 [External] 해시가 다른 파일은 받은 뒤에도 거부되고 캐시에서 지워진다", async () => {
    // Arrange: 캐시에 틀린 파일이 있고, 서버도 틀린 바이트를 준다
    const cache = tmp()
    writeFileSync(join(cache, "libmpv.zip"), "stale-wrong")
    const want = sha256("the-pinned-content")
    // Act
    const r = await ps([fnSource("Get-Verified")], `$CacheDir = ${psq(cache)}; Get-Verified ${psq(url)} '${want}' 'libmpv.zip'`)
    // Assert
    expect(r.error).toMatch(/^SHA-256 mismatch for libmpv\.zip \(got [0-9a-f]{64}, want [0-9a-f]{64}\)$/)
    expect(r.error).toContain(sha256(served))
    expect(existsSync(join(cache, "libmpv.zip"))).toBe(false)
  })

  it("T-sidecar-19 [Happy] 캐시 파일 해시가 맞으면 다시 받지 않는다(서버 없는 주소여도 통과)", async () => {
    const cache = tmp()
    writeFileSync(join(cache, "libmpv.zip"), "pinned")
    const r = await ps([fnSource("Get-Verified")], `$CacheDir = ${psq(cache)}; Get-Verified 'http://127.0.0.1:1/never' '${sha256("pinned")}' 'libmpv.zip'`)
    expect(r.error).toBeNull()
  })

  const assertDll = async (bytes: Buffer) => {
    const f = join(tmp(), "libmpv-2.dll")
    writeFileSync(f, bytes)
    return ps([fnSource("Assert-LgplLibMpv")], `Assert-LgplLibMpv ${psq(f)}; Write-Output 'PASSED'`)
  }

  it("T-sidecar-20 [Happy] LGPL 표시가 있는 DLL 은 통과한다", async () => {
    const r = await assertDll(fakeDll(LGPL, "-Dgpl=false"))
    expect(r.error).toBeNull()
    expect(r.out).toContain("PASSED")
  })

  it("T-sidecar-21 [Business rule] libzvbi 가 있으면 경고만 하고 통과한다(릴리스 전 결정 사항으로 남김)", async () => {
    const r = await assertDll(fakeDll(LGPL, "--enable-libzvbi"))
    expect(r.error).toBeNull()
    expect(r.out).toContain("PASSED")
  })

  it.each([
    ["--enable-gpl", /--enable-gpl, --enable-nonfree or -Dgpl=true/],
    ["--enable-nonfree", /--enable-gpl, --enable-nonfree or -Dgpl=true/],
    ["-Dgpl=true", /--enable-gpl, --enable-nonfree or -Dgpl=true/],
    ["libswscale license: GPL version 3 or later", /GPL\/nonfree license/],
    ["fftw_dft_indirect_register", /FFTW/],
    ["FFTW: assertion failed", /FFTW/],
  ])("T-sidecar-22 [Business rule] GPL/nonfree/FFTW 표지(%s)가 있는 DLL 은 거부한다", async (marker, message) => {
    const r = await assertDll(fakeDll(LGPL, marker))
    expect(r.error).toMatch(message)
    expect(r.out).not.toContain("PASSED")
  })

  it("T-sidecar-23 [Validation] LGPL 표시가 없는 DLL·PE 가 아닌 파일·너무 작은 파일은 거부한다", async () => {
    expect((await assertDll(fakeDll("no license info"))).error).toMatch(/does not report an LGPL license/)
    expect((await assertDll(Buffer.concat([Buffer.from("PK"), Buffer.alloc(1_100_000), Buffer.from(LGPL)]))).error).toMatch(/not a valid PE file/)
    expect((await assertDll(Buffer.from(`MZ${LGPL}`))).error).toMatch(/not a valid PE file/)
  })
})
