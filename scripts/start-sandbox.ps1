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
$uiConfigDir = Join-Path $repoRoot "config"

# 1. Build & stage
if (-not $SkipBuild) {
    Write-Host "=== Running build.ps1 ===" -ForegroundColor Cyan
    $stagingRoot = if (Test-Path "D:\tmp\ime_test") { "D:\tmp\ime_test_v4" } else { Join-Path $env:TEMP "cheime-stage" }
    & $buildScript -StagingRoot $stagingRoot
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Build/stage failed. Fix errors and re-run."
        exit 1
    }
}

# 2. Determine bundle path
$stagingRoot = if (Test-Path "D:\tmp\ime_test") { "D:\tmp\ime_test_v4" } else { Join-Path $env:TEMP "cheime-stage" }
$bundleDir = Get-ChildItem -Path $stagingRoot -Directory -Filter "cheime-bundle*" | Sort-Object LastWriteTime -Descending | Select-Object -First 1 -ExpandProperty FullName
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
$wsbContent = (Get-Content $template -Raw) `
    -replace '__BUNDLE_PATH__', $bundleDir `
    -replace '__BUNDLE_FOLDER__', $bundleName `
    -replace '__UI_CONFIG_PATH__', $uiConfigDir

$wsbFile = Join-Path $stagingRoot "CheIME.wsb"
Set-Content -Path $wsbFile -Value $wsbContent -NoNewline -Encoding utf8

Write-Host "`n=== Generated WSB config ===" -ForegroundColor Cyan
Write-Host "  Bundle: $bundleDir"
Write-Host "  Live UI config: $uiConfigDir\ui.yaml"
Write-Host "  WSB:    $wsbFile"
Write-Host "`nCleaning up old sandbox instances..." -ForegroundColor Yellow

# 4. Kill existing sandbox processes
$existing = Get-Process -Name "WindowsSandbox" -ErrorAction SilentlyContinue
if ($existing) {
    Write-Host "  Stopping $($existing.Count) running sandbox instance(s)..."
    $existing | Stop-Process -Force
    Start-Sleep -Seconds 2
}

# 5. Remove stale VHDX lock files if sandbox exited uncleanly
$sandboxVmDir = Join-Path $env:LOCALAPPDATA "Microsoft\Windows Sandbox"
if (Test-Path $sandboxVmDir) {
    Get-ChildItem -Path $sandboxVmDir -Filter "*.vhdx.lock" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
}

# 6. Launch Sandbox
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
