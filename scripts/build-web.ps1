# Build the wasm bundle and place wasm-bindgen output under web/pkg.
# Prereqs (one-time):
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli
#   cargo install wasm-opt   # optional, for size optimization
#
# Usage: powershell -ExecutionPolicy Bypass -File scripts/build-web.ps1

param(
    # Pass -Fast for the dev-wasm profile (no LTO, ~30s rebuilds). Default
    # is release-wasm (fully optimized, ~2-5min, what we deploy).
    [switch]$Fast
)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

$profile = if ($Fast) { "dev-wasm" } else { "release-wasm" }
Write-Host "==> cargo build ($profile)" -ForegroundColor Cyan
# Build the bin (not --lib): the lib crate-type is cdylib but exposes no
# wasm-bindgen exports, so --lib produces a dead-stripped 1MB stub. The bin
# entry point keeps main() reachable; wasm-bindgen wraps it for the browser.
cargo build --profile $profile --target wasm32-unknown-unknown --bin space_boosters
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$wasm = Join-Path $repo "target\wasm32-unknown-unknown\$profile\space_boosters.wasm"
if (-not (Test-Path $wasm)) {
    throw "wasm artifact not found at $wasm"
}

Write-Host "==> wasm-bindgen -> web/pkg" -ForegroundColor Cyan
$pkg = Join-Path $repo "web\pkg"
if (Test-Path $pkg) { Remove-Item -Recurse -Force $pkg }
wasm-bindgen --out-dir $pkg --target web --no-typescript $wasm
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Optional size pass — skipped silently if wasm-opt isn't on PATH.
$wasmOpt = Get-Command wasm-opt -ErrorAction SilentlyContinue
if ($wasmOpt) {
    $bg = Join-Path $pkg "space_boosters_bg.wasm"
    Write-Host "==> wasm-opt -Oz" -ForegroundColor Cyan
    wasm-opt -Oz $bg -o $bg
}

# Copy assets so Bevy's AssetServer can load them via fetch().
$assetsSrc = Join-Path $repo "assets"
if (Test-Path $assetsSrc) {
    $assetsDst = Join-Path $repo "web\assets"
    if (Test-Path $assetsDst) { Remove-Item -Recurse -Force $assetsDst }
    Write-Host "==> copy assets/ -> web/assets/" -ForegroundColor Cyan
    Copy-Item -Recurse $assetsSrc $assetsDst
}

$size = (Get-Item (Join-Path $pkg "space_boosters_bg.wasm")).Length
Write-Host ("==> done. wasm size: {0:N1} MB" -f ($size / 1MB)) -ForegroundColor Green
