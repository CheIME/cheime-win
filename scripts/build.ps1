# CheIME Windows build script
# Compiles all crates and copies artifacts to target/release/dist/

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path

Set-Location $projectRoot

Write-Host "=== Building CheIME Windows (release) ===" -ForegroundColor Cyan

cargo build --release
if ($LASTEXITCODE -ne 0) { throw "Build failed" }

$distDir = "target\release\dist"
New-Item -ItemType Directory -Force -Path $distDir | Out-Null

# Copy engine host
Copy-Item -Force "target\release\cheime-engine.exe" "$distDir\"
Write-Host "  cheime-engine.exe" -ForegroundColor Green

# Copy TIP DLL
Copy-Item -Force "target\release\cheime_tip.dll" "$distDir\cheime-tip.dll"
Write-Host "  cheime-tip.dll" -ForegroundColor Green

# Copy installer
Copy-Item -Force "target\release\cheime-installer.exe" "$distDir\"
Write-Host "  cheime-installer.exe" -ForegroundColor Green

# Copy dictionary data
$dictDest = "$distDir\data\dicts"
New-Item -ItemType Directory -Force -Path $dictDest | Out-Null
Copy-Item -Force "data\dicts\*" "$dictDest\"
Write-Host "  data/dicts/" -ForegroundColor Green

Write-Host ""
Write-Host "=== Build complete ===" -ForegroundColor Cyan
Write-Host "Artifacts: $distDir"
