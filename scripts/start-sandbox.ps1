# CheIME one-click Sandbox launcher
# ===================================
# Builds the release bundle, generates a .wsb config, and launches
# Windows Sandbox with the bundle mapped as a read-only folder.
#
# Usage:
#   .\scripts\start-sandbox.ps1              # normal run
#   .\scripts\start-sandbox.ps1 -SkipBuild   # re-launch with existing bundle

param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

# Resolve repo root
$repoRoot = Split-Path -Parent $PSScriptRoot
$buildScript = Join-Path $repoRoot "scripts\build.ps1"
$template = Join-Path $repoRoot "sandbox\CheIME.wsb.template"

# 1. Build & stage
if (-not $SkipBuild) {
    Write-Host "=== Running build.ps1 ===" -ForegroundColor Cyan
    & $buildScript
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Build/stage failed. Fix errors and re-run."
        exit 1
    }
}

# 2. Determine bundle path
# build.ps1 stages to D:\tmp\ime_test\cheime-bundle or %TEMP%\cheime-stage\cheime-bundle
$stagingRoot = if (Test-Path "D:\tmp\ime_test") { "D:\tmp\ime_test" } else { Join-Path $env:TEMP "cheime-stage" }
$bundleDir = Join-Path $stagingRoot "cheime-bundle"
if (-not (Test-Path $bundleDir)) {
    Write-Error "Bundle not found at $bundleDir. Run build.ps1 first."
    exit 1
}

# 3. Generate .wsb from template
$bundleName = Split-Path -Leaf $bundleDir
if (-not (Test-Path $template)) {
    Write-Error "WSB template not found: $template"
    exit 1
}
$wsbContent = (Get-Content $template -Raw) -replace '__BUNDLE_PATH__', $bundleDir -replace '__BUNDLE_FOLDER__', $bundleName

$wsbFile = Join-Path $stagingRoot "CheIME.wsb"
Set-Content -Path $wsbFile -Value $wsbContent -NoNewline -Encoding utf8

Write-Host "`n=== Generated WSB config ===" -ForegroundColor Cyan
Write-Host "  Bundle: $bundleDir"
Write-Host "  WSB:    $wsbFile"
Write-Host "`nLaunching Windows Sandbox..." -ForegroundColor Yellow

# 4. Launch Sandbox
$sandboxExe = "$env:SystemRoot\System32\WindowsSandbox.exe"
if (-not (Test-Path $sandboxExe)) {
    Write-Error "Windows Sandbox not found at $sandboxExe."
    Write-Error "Install via: Turn Windows features on or off → Windows Sandbox"
    exit 1
}
Start-Process -FilePath $sandboxExe -ArgumentList $wsbFile

Write-Host "`n==========================================" -ForegroundColor Green
Write-Host "  SANDBOX LAUNCHED" -ForegroundColor Green
Write-Host "==========================================" -ForegroundColor Green
Write-Host ""
Write-Host "Inside Sandbox, the guest-run.ps1 script will:"
Write-Host "  1. Install CheIME TIP"
Write-Host "  2. Start the engine"
Write-Host "  3. Run registration and profile probes"
Write-Host "  4. Prompt you to test in Notepad"
Write-Host "  5. Clean up (stop engine, unregister, remove files)"
