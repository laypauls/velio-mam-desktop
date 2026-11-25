<#
 Builds the player worker (crates/mam-player-worker, release profile) and stages it for Tauri's externalBin:
   src-tauri/binaries/mam-player-worker-<triple>.exe  ->  installed as <install dir>/mam-player-worker.exe (next to velio.exe)

 Run automatically as the beforeBuildCommand of src-tauri/tauri.sidecars.conf.json (before the app's own cargo build, whose
 build script requires every externalBin file to exist). Run it by hand only to inspect the result.
 The libmpv DLL is not part of this script - scripts/fetch-sidecars.ps1 stages it.

 Usage: pwsh scripts/build-player-worker.ps1      (Windows PowerShell 5.1 works too)
#>
$ErrorActionPreference = 'Stop'
$Triple = 'x86_64-pc-windows-msvc'
$Root = Split-Path -Parent $PSScriptRoot
$Bin = Join-Path $Root 'src-tauri/binaries'
Set-Location $Root

# --locked: a release worker must be built from exactly the committed Cargo.lock (build reproducibility).
cargo build --release --locked -p mam-player-worker
if ($LASTEXITCODE -ne 0) { throw "cargo build -p mam-player-worker failed (exit $LASTEXITCODE)" }

# The target dir is configurable (.cargo/config.toml, CARGO_TARGET_DIR): ask cargo instead of assuming ./target.
$meta = cargo metadata --format-version 1 --no-deps --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed' }
$exe = Join-Path $meta.target_directory "release/mam-player-worker.exe"
if (-not (Test-Path $exe)) { throw "worker build output not found: $exe" }

New-Item -ItemType Directory -Force -Path $Bin | Out-Null
Copy-Item $exe (Join-Path $Bin "mam-player-worker-$Triple.exe") -Force
Write-Host ('mam-player-worker staged: {0:N1} MB' -f ((Get-Item (Join-Path $Bin "mam-player-worker-$Triple.exe")).Length / 1MB))
