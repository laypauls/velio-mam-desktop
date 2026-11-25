<#
 Downloads the bundled sidecar binaries (pinned URL + SHA-256) into src-tauri/binaries/.
 Binaries are gitignored; run this before `pnpm tauri build --config src-tauri/tauri.sidecars.conf.json`.

   FFmpeg/FFprobe  BtbN/FFmpeg-Builds  n8.1 LGPL *shared* build (no --enable-gpl / --enable-nonfree)
   whisper-cli     ggml-org/whisper.cpp official release whisper-bin-x64.zip (CPU)
   libmpv          mpv-player/mpv official release v0.42.0 libmpv-...-x86_64-w64-mingw32-lgpl.zip: only libmpv-2.dll + LICENSE.LGPL
                   (LGPL build; the script refuses a DLL whose embedded FFmpeg reports GPL/nonfree or that contains FFTW)
   mam-player-worker.exe is NOT downloaded: scripts/build-player-worker.ps1 builds it from this repo (tauri.sidecars.conf.json beforeBuildCommand).

 Layout (Tauri externalBin = target-triple suffix, resources = DLLs next to the exe):
   binaries/ffmpeg-<triple>.exe, ffprobe-<triple>.exe, whisper-cli-<triple>.exe
   binaries/dlls/*.dll          (FFmpeg + ggml/whisper DLLs, installed next to the exe)
   binaries/libmpv/            (libmpv-2.dll, LICENSE.LGPL -> <install dir>/libmpv/; the worker loads <exe dir>/libmpv/libmpv-2.dll)
   binaries/licenses/*          (FFmpeg LICENSE.txt, whisper.cpp LICENSE, GPL-3.0.txt, GPL-2.0.txt)

 Usage: pwsh scripts/fetch-sidecars.ps1 [-CacheDir <dir>] [-Force]
#>
param(
  [string]$CacheDir = "",
  [switch]$Force
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$Triple = 'x86_64-pc-windows-msvc'
$Root = Split-Path -Parent $PSScriptRoot
$Bin = Join-Path $Root 'src-tauri/binaries'
if (-not $CacheDir) { $CacheDir = Join-Path $Bin '.cache' }

# Lowercase hex SHA-256 of a file via .NET. Deliberately not Get-FileHash: that cmdlet lives in Microsoft.PowerShell.Utility, which
# Windows PowerShell 5.1 failed to autoload on the CI runner (PSModulePath inherited from PowerShell 7), while .NET needs no module.
function Get-Sha256($Path) {
  $sha = [Security.Cryptography.SHA256]::Create()
  $fs = [IO.File]::OpenRead((Convert-Path $Path))
  try { return ([BitConverter]::ToString($sha.ComputeHash($fs)) -replace '-', '').ToLower() }
  finally { $fs.Dispose(); $sha.Dispose() }
}

$FfmpegUrl = 'https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-30-13-08/ffmpeg-n8.1.3-9-g29e619e767-win64-lgpl-shared-8.1.zip'
$FfmpegSha = '3e47bda1607740550141e37c0e49d1e5182b34699f15adfd137ee266d346811a'
$WhisperUrl = 'https://github.com/ggml-org/whisper.cpp/releases/download/b5454/whisper-bin-x64.zip'
$WhisperSha = '6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb'

$MpvUrl = 'https://github.com/mpv-player/mpv/releases/download/v0.42.0/libmpv-v0.42.0-x86_64-w64-mingw32-lgpl.zip'
$MpvSha = '8e5f77430efc53ef175804ba6618527e3af0448a0dcabf04d3cc72659d1a6fce'
$MpvDllName = 'libmpv-2.dll'
$MpvLicenseName = 'LICENSE.LGPL'

# The FFmpeg build is distributed under GPL-3.0-or-later (BtbN links GPL-2.0-or-later FFTW 3 into avformat via Chromaprint; see
# THIRD_PARTY_NOTICES.md 1.1), so the GPL-3.0 and GPL-2.0 texts ship next to
# ffmpeg-LICENSE.txt. It is copied from the pinned file in the repo (licenses/README.md lists source + SHA-256), not downloaded.
$GplText = Join-Path $Root 'licenses/GPL-3.0.txt'
if (-not (Test-Path $GplText)) { throw "missing pinned license text: $GplText" }
$GplSha = (Get-Sha256 $GplText)
$Gpl2Text = Join-Path $Root 'licenses/GPL-2.0.txt'
if (-not (Test-Path $Gpl2Text)) { throw "missing pinned license text: $Gpl2Text" }
$GplSha += (Get-Sha256 $Gpl2Text)

$Stamp = Join-Path $Bin '.fetched'
$Want = "$FfmpegSha;$WhisperSha;$MpvSha;$GplSha"
if (-not $Force -and (Test-Path $Stamp) -and ((Get-Content $Stamp -Raw).Trim() -eq $Want) -and
    (Test-Path (Join-Path $Bin "ffmpeg-$Triple.exe")) -and (Test-Path (Join-Path $Bin "whisper-cli-$Triple.exe")) -and
    (Test-Path (Join-Path $Bin "libmpv/$MpvDllName")) -and (Test-Path (Join-Path $Bin "libmpv/$MpvLicenseName")) -and -not (Test-Path (Join-Path $Bin 'libvlc')) -and (Test-Path (Join-Path $Bin 'licenses/GPL-3.0.txt')) -and (Test-Path (Join-Path $Bin 'licenses/GPL-2.0.txt'))) {
  Write-Host 'sidecars up to date'; exit 0
}

function Get-Verified($Url, $Sha, $Name) {
  New-Item -ItemType Directory -Force -Path $CacheDir | Out-Null
  $dest = Join-Path $CacheDir $Name
  if (-not (Test-Path $dest) -or (Get-Sha256 $dest) -ne $Sha) {
    Write-Host "downloading $Url"
    Invoke-WebRequest -Uri $Url -OutFile $dest -UseBasicParsing
  }
  $got = (Get-Sha256 $dest)
  if ($got -ne $Sha) { Remove-Item $dest -Force; throw "SHA-256 mismatch for $Name (got $got, want $Sha)" }
  return $dest
}

# Copies only libmpv-2.dll and LICENSE.LGPL out of the (already hash-verified) zip into $Dest. Entries are matched by leaf name
# and written to fixed names, so nothing else in the archive (include/, .jar, .dll.a) and no archive-supplied path is used.
function Expand-LibMpv($Zip, $Dest, $DllName, $LicenseName) {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  if (Test-Path $Dest) { Remove-Item $Dest -Recurse -Force }
  New-Item -ItemType Directory -Force -Path $Dest | Out-Null
  $za = [IO.Compression.ZipFile]::OpenRead($Zip)
  try {
    foreach ($name in @($DllName, $LicenseName)) {
      $entry = $za.Entries | Where-Object { $_.Name -eq $name } | Select-Object -First 1
      if (-not $entry) { throw "$name not found in libmpv archive" }
      [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $Dest $name), $true)
    }
  } finally { $za.Dispose() }
}

# The distributed libmpv must be the LGPL build: refuse a DLL whose embedded FFmpeg/mpv configuration reports GPL or nonfree,
# and one that contains FFTW (GPL-2.0-or-later; the BtbN chromaprint switched from FFTW to kissfft on 2026-10-28).
# libzvbi (GPL-2.0-only packet-830.c/pdc.c) is only warned about - see THIRD_PARTY_NOTICES.md 1.3.
function Assert-LgplLibMpv($DllPath) {
  $bytes = [IO.File]::ReadAllBytes($DllPath)
  if ($bytes.Length -lt 1MB -or $bytes[0] -ne 0x4D -or $bytes[1] -ne 0x5A) { throw "libmpv DLL is not a valid PE file: $DllPath" }
  $txt = [Text.Encoding]::GetEncoding(28591).GetString($bytes)
  $ord = [StringComparison]::Ordinal
  if ($txt.IndexOf('--enable-gpl', $ord) -ge 0 -or $txt.IndexOf('--enable-nonfree', $ord) -ge 0 -or $txt.IndexOf('-Dgpl=true', $ord) -ge 0) {
    throw 'libmpv build reports --enable-gpl, --enable-nonfree or -Dgpl=true; refusing to bundle'
  }
  if ($txt.IndexOf('license: GPL', $ord) -ge 0 -or $txt.IndexOf('license: nonfree', $ord) -ge 0) {
    throw 'libmpv embedded FFmpeg reports a GPL/nonfree license; refusing to bundle'
  }
  if ($txt.IndexOf('license: LGPL', $ord) -lt 0) { throw 'libmpv embedded FFmpeg does not report an LGPL license; refusing to bundle' }
  if ($txt.IndexOf('fftw', [StringComparison]::OrdinalIgnoreCase) -ge 0) { throw 'libmpv contains FFTW (GPL-2.0-or-later); refusing to bundle' }
  if ($txt.IndexOf('--enable-libzvbi', $ord) -ge 0) {
    Write-Warning 'libmpv links libzvbi (src/packet-830.c and src/pdc.c are GPL-2.0-only; BtbN FFmpeg-Builds issue 694). Resolve before release - see THIRD_PARTY_NOTICES.md 1.3.'
  }
}

# Refuse a GPL/nonfree FFmpeg: the distributed build must be LGPL. Fails closed: if the exe cannot run (its shared DLLs live in
# $DllDir, not next to the exe, so $DllDir is prepended to PATH for the call only) or prints no version/configuration, the build is
# rejected rather than silently passing an unchecked binary.
function Assert-LgplFfmpeg($Exe, $DllDir) {
  $oldPath = $env:PATH
  $oldEap = $ErrorActionPreference
  try {
    $env:PATH = "$DllDir;$oldPath"
    $ErrorActionPreference = 'Continue' # Windows PowerShell 5.1 turns native stderr into terminating errors under 'Stop'
    $ver = & $Exe -version 2>&1 | Out-String
    $code = $LASTEXITCODE
  } catch {
    throw "FFmpeg could not be run for the license check: $($_.Exception.Message)"
  } finally {
    $env:PATH = $oldPath
    $ErrorActionPreference = $oldEap
  }
  if ($code -ne 0) { throw "FFmpeg -version exited with code $code (missing DLLs?); cannot verify the LGPL build: $ver" }
  if ($ver -notmatch 'ffmpeg version' -or $ver -notmatch 'configuration:') { throw "FFmpeg -version printed no version/configuration; cannot verify the LGPL build: $ver" }
  if ($ver -match '--enable-gpl' -or $ver -match '--enable-nonfree') {
    throw 'FFmpeg build reports --enable-gpl or --enable-nonfree; refusing to bundle'
  }
}

$ff = Get-Verified $FfmpegUrl $FfmpegSha 'ffmpeg-lgpl-shared.zip'
$wh = Get-Verified $WhisperUrl $WhisperSha 'whisper-bin-x64.zip'
$mpv = Get-Verified $MpvUrl $MpvSha 'libmpv-v0.42.0-x86_64-w64-mingw32-lgpl.zip'

$tmp = Join-Path $CacheDir 'extract'
if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }
New-Item -ItemType Directory -Force -Path "$tmp/ff", "$tmp/wh" | Out-Null
Expand-Archive -Path $ff -DestinationPath "$tmp/ff" -Force
Expand-Archive -Path $wh -DestinationPath "$tmp/wh" -Force

$dll = Join-Path $Bin 'dlls'; $lic = Join-Path $Bin 'licenses'
foreach ($d in @($dll, $lic)) { if (Test-Path $d) { Remove-Item $d -Recurse -Force }; New-Item -ItemType Directory -Force -Path $d | Out-Null }

$ffBin = Get-ChildItem "$tmp/ff" -Recurse -Filter ffmpeg.exe | Select-Object -First 1
if (-not $ffBin) { throw 'ffmpeg.exe not found in archive' }
$ffDir = $ffBin.Directory.FullName
Copy-Item (Join-Path $ffDir 'ffmpeg.exe') (Join-Path $Bin "ffmpeg-$Triple.exe") -Force
Copy-Item (Join-Path $ffDir 'ffprobe.exe') (Join-Path $Bin "ffprobe-$Triple.exe") -Force
Copy-Item (Join-Path $ffDir '*.dll') $dll -Force
$ffRoot = Split-Path -Parent $ffDir
if (Test-Path (Join-Path $ffRoot 'LICENSE.txt')) { Copy-Item (Join-Path $ffRoot 'LICENSE.txt') (Join-Path $lic 'ffmpeg-LICENSE.txt') }

$whCli = Get-ChildItem "$tmp/wh" -Recurse -Filter whisper-cli.exe | Select-Object -First 1
if (-not $whCli) { throw 'whisper-cli.exe not found in archive' }
Copy-Item $whCli.FullName (Join-Path $Bin "whisper-cli-$Triple.exe") -Force
Copy-Item (Join-Path $whCli.Directory.FullName '*.dll') $dll -Force

Assert-LgplFfmpeg (Join-Path $Bin "ffmpeg-$Triple.exe") $dll
Write-Host 'FFmpeg LGPL check passed'

# libmpv: only the DLL and its license, laid out as libmpv/{libmpv-2.dll,LICENSE.LGPL} (-> <install dir>/libmpv/).
$mpvDir = Join-Path $Bin 'libmpv'
Expand-LibMpv $mpv $mpvDir $MpvDllName $MpvLicenseName
try { Assert-LgplLibMpv (Join-Path $mpvDir $MpvDllName) }
catch { Remove-Item $mpvDir -Recurse -Force; throw }
# A previous (libVLC) layout must not linger in the bundle.
$oldVlc = Join-Path $Bin 'libvlc'
if (Test-Path $oldVlc) { Remove-Item $oldVlc -Recurse -Force }
Copy-Item $GplText (Join-Path $lic 'GPL-3.0.txt') -Force
Copy-Item $Gpl2Text (Join-Path $lic 'GPL-2.0.txt') -Force

Remove-Item $tmp -Recurse -Force
Set-Content -Path $Stamp -Value $Want
Write-Host 'sidecars ready:'; Get-ChildItem $Bin -File | Where-Object { $_.Name -notlike '.*' } | ForEach-Object { '{0}  {1:N1} MB' -f $_.Name, ($_.Length/1MB) }
Write-Host ('libmpv: {0:N1} MB' -f ((Get-Item (Join-Path $mpvDir $MpvDllName)).Length/1MB))
Write-Host ('dlls: {0} files, {1:N1} MB' -f (Get-ChildItem $dll).Count, ((Get-ChildItem $dll | Measure-Object Length -Sum).Sum/1MB))
