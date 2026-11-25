// `pnpm dev:temp` — D:\mam-desktop\library 생성 후 개발 데이터를 D:\mam-desktop\MAMData 로 분리해 `tauri dev` 를 실행한다.
// C: 여유 공간 부족으로 이 개발 PC 의 D: 에 고정했다 (D: 가 없는 PC 에서는 이 경로를 바꿔야 한다).
// 다른 `tauri dev`(Vite :1420, 단일 인스턴스 뮤텍스)가 떠 있으면 실행할 수 없다.
import { spawn, spawnSync } from "node:child_process"
import { createConnection } from "node:net"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { hasCdpFlag, mergeBrowserArgs, resolveCdpPort } from "./dev-temp-cdp.mjs"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const devRoot = "D:\\mam-desktop"
const library = join(devRoot, "library")
const dataDir = join(devRoot, "MAMData")

const gen = spawnSync(process.execPath, [join(root, "scripts", "dev-temp-library.mjs"), library], { stdio: "inherit" })
if (gen.status !== 0) process.exit(gen.status ?? 1)

// Vite 는 localhost 를 IPv6(::1) 로만 열 수 있어 두 주소를 모두 확인한다
const busyOn = (port, host) =>
  new Promise((res) => {
    const s = createConnection({ port, host })
    s.once("connect", () => (s.destroy(), res(true)))
    s.once("error", () => res(false))
  })
const portBusy = (await busyOn(1420, "127.0.0.1")) || (await busyOn(1420, "::1"))
if (portBusy) {
  console.error("Port 1420 is in use - another `tauri dev` is probably running. Stop it first (single instance per user).")
  process.exit(1)
}

// WebView2 원격 디버깅(CDP) — 기본 19333, MAM_DEV_CDP_PORT 로 재지정(0 = 끔)
let cdpPort
try {
  cdpPort = resolveCdpPort(process.env.MAM_DEV_CDP_PORT)
} catch (e) {
  console.error(e.message)
  process.exit(1)
}
const userArgs = process.env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
const env = { ...process.env, MAM_DATA_DIR: dataDir, MAM_DEV_SEED_FOLDER: library }
let cdpLine = "CDP: disabled (MAM_DEV_CDP_PORT=0)"
if (hasCdpFlag(userArgs)) {
  cdpLine = `CDP: using existing WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=${userArgs}`
} else if (cdpPort !== null) {
  if (await busyOn(cdpPort, "127.0.0.1")) {
    console.error(`CDP port ${cdpPort} is in use - WebView2 would start without remote debugging. Free it or set MAM_DEV_CDP_PORT to another port (0 = disable).`)
    process.exit(1)
  }
  env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = mergeBrowserArgs(userArgs, cdpPort)
  cdpLine = `CDP: http://127.0.0.1:${cdpPort}/json/version`
}

console.log(`MAM_DATA_DIR=${dataDir}\nMAM_DEV_SEED_FOLDER=${library}\n${cdpLine}`)
// Windows 에서 pnpm 은 .cmd 셸이라 shell 이 필요하다. 환경 변수는 인자가 아니라 env 로 넘긴다.
const child = spawn("pnpm", ["tauri", "dev"], {
  cwd: root,
  stdio: "inherit",
  shell: true,
  env,
})
child.on("exit", (code, signal) => process.exit(code ?? (signal ? 1 : 0)))
