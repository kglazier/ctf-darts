# Build the aarch64-android .so and stage it for gradle, then bundle the
# release AAB.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts/build-android.ps1
#   powershell -ExecutionPolicy Bypass -File scripts/build-android.ps1 -SkipBundle
#
# Requirements (one-time): cargo-ndk, ANDROID_NDK_HOME / ANDROID_NDK_ROOT.
# `--platform 24` pins the link-target API so `nix`'s getifaddrs/freeifaddrs
# resolve — the NDK's libc stubs only include them at API 24+. Without
# this flag cargo-ndk targets API 21 and the link fails. NB: must be the
# long form; `-p` collides with cargo's --package and panics cargo-ndk.

param(
    [switch]$SkipBundle
)

$ErrorActionPreference = "Stop"

$repo = Split-Path -Parent $PSScriptRoot
Set-Location $repo

Write-Host "==> cargo ndk build (release, arm64-v8a, API 24)" -ForegroundColor Cyan
cargo ndk --platform 24 -t arm64-v8a -o android/app/src/main/jniLibs build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($SkipBundle) {
    Write-Host "==> -SkipBundle set; stopping after .so" -ForegroundColor Yellow
    exit 0
}

Write-Host "==> gradle bundleRelease" -ForegroundColor Cyan
Push-Location android
try {
    .\gradlew.bat bundleRelease
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
} finally {
    Pop-Location
}

$aab = Get-ChildItem android\app\build\outputs\bundle\release\*.aab -ErrorAction SilentlyContinue | Select-Object -First 1
if ($aab) {
    Write-Host ("==> done. AAB: {0} ({1:N1} MB)" -f $aab.FullName, ($aab.Length / 1MB)) -ForegroundColor Green
}
