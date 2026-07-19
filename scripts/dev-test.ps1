# CheIME dev-test: build, install, start engine for testing
param(
    [switch]$SkipBuild,
    [switch]$SkipInstall
)

$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not $SkipBuild) {
    Write-Host "=== Step 1: Build ===" -ForegroundColor Cyan
    & "$projectRoot\build.ps1"
    if ($LASTEXITCODE -ne 0) { exit 1 }
}

if (-not $SkipInstall) {
    Write-Host ""
    Write-Host "=== Step 2: Install ===" -ForegroundColor Cyan
    & "$projectRoot\install.ps1"
}

Write-Host ""
Write-Host "=== Step 3: Start Engine ===" -ForegroundColor Cyan
$dataDir = "$env:LOCALAPPDATA\CheIME\data\dicts"
$engineExe = "$env:LOCALAPPDATA\CheIME\bin\cheime-engine.exe"

if (-not (Test-Path $engineExe)) {
    $engineExe = "$projectRoot\target\release\cheime-engine.exe"
    $dataDir = "$projectRoot\data\dicts"
}

Write-Host "Engine: $engineExe"
Write-Host "Dicts:  $dataDir"
Write-Host ""
Write-Host "Starting engine in a new window..." -ForegroundColor Yellow

# Start engine in a new console window
Start-Process powershell -ArgumentList "-NoExit", "-Command", "& '$engineExe' --dict-dir '$dataDir'"

Write-Host ""
Write-Host "=== Dev test setup complete ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "To test:"
Write-Host "  1. Open Notepad"
Write-Host "  2. Switch to CheIME input method (Win+Space)"
Write-Host "  3. Type pinyin to see Chinese characters"
Write-Host ""
Write-Host "To test in stdin mode:"
Write-Host "  echo '{...json...}' | $engineExe --stdin"
