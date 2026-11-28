// `pnpm dev:temp` 보조 — 개발 실행(target/debug)에는 externalBin 오버레이가 적용되지 않아 whisper-cli 가 없다.
// 앱은 whisper-cli 를 `<exe 폴더>` -> PATH 순으로 찾으므로(crates/mam-ai/src/local/stt/tool.rs) 동봉본을 exe 옆에 복사한다.
// ffmpeg 는 복사하지 않는다 — 같은 규칙(exe 폴더 -> PATH)이지만 PATH 의 ffmpeg 로 충분하고 DLL 의존이 커서 범위 밖.
// 원본이 없어도 개발 실행은 막지 않는다(경고만).
import { copyFileSync, existsSync, mkdirSync, readdirSync, statSync } from "node:fs"
import { join } from "node:path"

const WHISPER_SRC = "whisper-cli-x86_64-pc-windows-msvc.exe"
const WHISPER_DST = "whisper-cli.exe"
const isWhisperDll = (n) => /^(whisper|ggml.*)\.dll$/i.test(n)

/** cargo 타깃 폴더: CARGO_TARGET_DIR(상대 경로면 root 기준) 또는 `<root>/target`. */
export function resolveTargetDir(root, env = process.env) {
  const t = env.CARGO_TARGET_DIR
  return t ? join(root, t) : join(root, "target")
}

function needsCopy(src, dst) {
  if (!existsSync(dst)) return true
  const s = statSync(src)
  const d = statSync(dst)
  return s.size !== d.size || s.mtimeMs > d.mtimeMs
}

/** 필요한 파일만 복사하고 복사한 대상 파일명 목록을 돌려준다. 원본이 없으면 경고만 하고 빈 목록. */
export function ensureDevSidecars({ root, env = process.env, warn = console.warn, profile = "debug" }) {
  const bin = join(root, "src-tauri", "binaries")
  const exe = join(bin, WHISPER_SRC)
  if (!existsSync(exe)) {
    warn(`[dev-sidecars] ${exe} 없음 - 자막(STT)이 건너뛰어집니다. scripts/fetch-sidecars.ps1 로 받으세요.`)
    return []
  }
  const outDir = join(resolveTargetDir(root, env), profile)
  mkdirSync(outDir, { recursive: true })
  const pairs = [[exe, join(outDir, WHISPER_DST)]]
  const dlls = join(bin, "dlls")
  if (existsSync(dlls)) {
    for (const n of readdirSync(dlls).filter(isWhisperDll)) pairs.push([join(dlls, n), join(outDir, n)])
  } else {
    warn(`[dev-sidecars] ${dlls} 없음 - whisper-cli 가 DLL 을 못 찾을 수 있습니다. scripts/fetch-sidecars.ps1 참고.`)
  }
  const copied = []
  for (const [src, dst] of pairs) {
    if (!needsCopy(src, dst)) continue
    try {
      copyFileSync(src, dst)
      copied.push(dst)
    } catch (e) {
      warn(`[dev-sidecars] 복사 실패 ${dst}: ${e.message} (실행 중인 앱이 잠갔을 수 있음)`)
    }
  }
  return copied
}
