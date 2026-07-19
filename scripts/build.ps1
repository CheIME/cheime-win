# CheIME Windows build script
# Run from project root

$ErrorActionPreference = "Stop"

$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $projectRoot
Set-Location ..

Write-Host "=== Building CheIME Windows (release) ===" -ForegroundColor Cyan

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "Build failed" }

Write-Host ""
Write-Host "=== Build complete ===" -ForegroundColor Cyan
Write-Host "Artifacts: $(Resolve-Path target\release)"
Write-Host "  cheime-engine.exe"
Write-Host "  cheime_tip.dll"
Write-Host "  cheime-installer.exe"
