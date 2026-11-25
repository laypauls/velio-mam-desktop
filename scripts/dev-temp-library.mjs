// 개발용 테스트 관리 폴더(temp/library) 생성 — 동봉 ffmpeg 사이드카(lavfi 합성 소스)로 이미지·영상·음성을 만든다.
// 재실행해도 기존 파일을 덮어쓰지 않고 누락분만 만든다. 외부 의존성 없음(Node 내장만).
// 사용: node scripts/dev-temp-library.mjs [출력 폴더, 기본 <repo>/temp/library]
import { spawnSync } from "node:child_process"
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs"
import { dirname, join, delimiter, extname, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..")
const lib = resolve(process.argv[2] ?? join(root, "temp", "library"))
const bin = join(root, "src-tauri", "binaries")
const ffmpeg = join(bin, "ffmpeg-x86_64-pc-windows-msvc.exe")
const heic = join(root, "crates", "mam-media", "tests", "fixtures", "tiny.heic")
// ffmpeg 가 같은 폴더가 아닌 dlls/ 의 공유 라이브러리를 찾도록 PATH 에 덧붙인다
const env = { ...process.env, PATH: `${join(bin, "dlls")}${delimiter}${process.env.PATH ?? ""}` }
// 비 ASCII 인자를 ffmpeg 에 넘기지 않도록 ASCII 임시 이름으로 만든 뒤 최종 이름으로 복사한다
const work = join(dirname(lib), ".gen-work")

if (!existsSync(ffmpeg)) {
  console.error(`ffmpeg sidecar not found: ${ffmpeg}\n(scripts/fetch-sidecars.ps1 로 받는다)`)
  process.exit(1)
}

const created = []
const skipped = []
const failed = []
let seq = 0

function has(rel) {
  return existsSync(join(lib, rel))
}

function place(rel, produce) {
  if (has(rel)) {
    skipped.push(rel)
    return
  }
  const dest = join(lib, rel)
  mkdirSync(dirname(dest), { recursive: true })
  produce(dest)
  created.push(rel)
}

/** lavfi 입력(`src`) -> 출력. `out` 은 ffmpeg 출력 옵션 배열. */
function gen(rel, src, out) {
  place(rel, (dest) => {
    mkdirSync(work, { recursive: true })
    const tmp = join(work, `g${++seq}${extname(rel)}`)
    const args = ["-hide_banner", "-loglevel", "error", "-y"]
    for (const s of Array.isArray(src) ? src : [src]) args.push("-f", "lavfi", "-i", s)
    args.push(...out, tmp)
    const r = spawnSync(ffmpeg, args, { env, encoding: "utf8" })
    if (r.status !== 0 || !existsSync(tmp)) {
      failed.push(rel)
      console.error(`FAILED ${rel}\n${r.stderr || r.error}`)
      throw new Error("ffmpeg failed")
    }
    copyFileSync(tmp, dest)
    rmSync(tmp)
  })
}

function safe(fn) {
  try {
    fn()
  } catch {
    /* 실패는 failed 에 기록됨 — 나머지는 계속 만든다 */
  }
}

const img = (rel, size, source, extra = []) => safe(() => gen(rel, `${source}=size=${size}`, ["-frames:v", "1", ...extra]))
const h264 = ["-c:v", "libopenh264", "-b:v", "1500k", "-pix_fmt", "yuv420p"]
const tone = (hz, sec) => `sine=frequency=${hz}:duration=${sec}`
const vid = (rel, size, source, sec, rate, codec, audio) =>
  safe(() => {
    const src = [`${source}=size=${size}:rate=${rate}`]
    if (audio) src.push(`sine=frequency=${audio}`)
    gen(rel, src, ["-t", String(sec), ...codec, ...(audio ? ["-c:a", "aac", "-b:a", "96k"] : ["-an"])])
  })
const aud = (rel, hz, sec, codec) => safe(() => gen(rel, tone(hz, sec), codec))

// ---- 이미지 ----
img("images/photos/sunset_1920x1080.jpg", "1920x1080", "testsrc2", ["-q:v", "3"])
img("images/photos/portrait_1080x1920.jpg", "1080x1920", "testsrc", ["-q:v", "3"])
img("images/photos/large_4000x3000.jpg", "4000x3000", "mandelbrot", ["-q:v", "4"])
img("images/photos/small_320x240.jpg", "320x240", "smptebars", ["-q:v", "5"])
img("images/graphics/logo_512.png", "512x512", "rgbtestsrc")
img("images/graphics/gradient_800x600.webp", "800x600", "testsrc2", ["-c:v", "libwebp", "-quality", "80"])
img("images/graphics/mono_tall_600x1600.png", "600x1600", "yuvtestsrc")
safe(() => gen("images/graphics/animated.gif", "testsrc=size=240x180:rate=10:duration=3", []))
img("images/scans/scan_page_640x480.tiff", "640x480", "testsrc2")
img("images/scans/old_bitmap_320x240.bmp", "320x240", "rgbtestsrc")
if (!has("images/scans/sample.heic")) place("images/scans/sample.heic", (d) => copyFileSync(heic, d))

// ---- 영상 (2~5초) ----
vid("videos/clips/clip_h264_audio_1280x720.mp4", "1280x720", "testsrc2", 4, 25, h264, 440)
vid("videos/clips/clip_vp9_640x360.webm", "640x360", "testsrc", 3, 25, ["-c:v", "libvpx-vp9", "-b:v", "300k", "-deadline", "realtime", "-cpu-used", "8"], 0)
vid("videos/clips/clip_portrait_720x1280.mov", "720x1280", "rgbtestsrc", 2, 25, h264, 0)
vid("videos/archive/clip_h264_960x540.mkv", "960x540", "smptebars", 3, 25, h264, 0)
vid("videos/archive/clip_silent_5s.mp4", "640x360", "mandelbrot", 5, 15, h264, 0)

// ---- 음성 ----
aud("audio/music/tone_440hz.mp3", 440, 5, ["-c:a", "libmp3lame", "-b:a", "128k"])
aud("audio/music/tone_523hz.wav", 523, 3, ["-c:a", "pcm_s16le", "-ar", "22050"])
aud("audio/music/tone_660hz.flac", 660, 4, ["-c:a", "flac"])
aud("audio/voice/voice_note_880hz.m4a", 880, 3, ["-c:a", "aac", "-b:a", "96k"])

// ---- 한글·공백·긴 파일명, 깊은 폴더 ----
img("여름 휴가/해변 사진 01.jpg", "1280x720", "testsrc2", ["-q:v", "4"])
img("여름 휴가/해변 사진 02 (편집본).png", "800x600", "testsrc")
vid("여름 휴가/2026 가족 여행/오전 일정/물놀이 영상 - 첫째 날.mp4", "640x360", "testsrc", 3, 25, h264, 330)
aud("여름 휴가/2026 가족 여행/오전 일정/파도 소리 녹음.mp3", 300, 3, ["-c:a", "libmp3lame", "-b:a", "96k"])
const longName = `아주 긴 파일 이름 테스트 ${"가나다라마바사 ".repeat(10)}끝`.trim()
img(`긴 이름 모음/${longName}.jpg`, "640x480", "testsrc2", ["-q:v", "5"])
img("공백  많은   폴더/  앞뒤 공백 없음 + 특수문자 &#(1).jpg", "640x480", "rgbtestsrc", ["-q:v", "5"])

// ---- 완전 중복 (다른 폴더에 같은 내용) ----
if (has("images/photos/sunset_1920x1080.jpg")) {
  place("중복 테스트/원본 복사본/sunset_1920x1080.jpg", (d) => copyFileSync(join(lib, "images/photos/sunset_1920x1080.jpg"), d))
}

// ---- 제외 대상 (미디어 아님) ----
place("excluded/notes.txt", (d) => writeFileSync(d, "개발용 메모 - 미디어가 아니므로 색인 대상이 아니다.\n"))
place("excluded/minimal.pdf", (d) =>
  writeFileSync(
    d,
    "%PDF-1.1\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n" +
      "3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]>>endobj\ntrailer<</Root 1 0 R/Size 4>>\n%%EOF\n",
  ),
)
place("excluded/empty.bin", (d) => writeFileSync(d, ""))

rmSync(work, { recursive: true, force: true })

// ---- 요약 ----
function walk(dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name)
    if (e.isDirectory()) walk(p, out)
    else out.push(p)
  }
  return out
}
const files = walk(lib)
const bytes = files.reduce((n, f) => n + statSync(f).size, 0)
const byExt = {}
for (const f of files) byExt[extname(f).toLowerCase() || "(none)"] = (byExt[extname(f).toLowerCase() || "(none)"] ?? 0) + 1
console.log(`library: ${lib}`)
console.log(`created ${created.length}, kept ${skipped.length}, failed ${failed.length}`)
console.log(`total ${files.length} files, ${(bytes / 1024 / 1024).toFixed(1)} MB`)
console.log(Object.entries(byExt).sort().map(([k, v]) => `${k}:${v}`).join("  "))
if (failed.length) {
  console.error(`failed: ${failed.map((f) => relative(lib, join(lib, f))).join(", ")}`)
  process.exit(1)
}
