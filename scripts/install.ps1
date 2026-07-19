# CheIME Windows install script
# Run from project root. Requires admin for HKLM registration.

$ErrorActionPreference = "Stop"

# Always resolve to project root regardless of where script is invoked
$projectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $projectRoot
Set-Location ..

$releaseDir = "target\release"
if (-not (Test-Path "$releaseDir\cheime-engine.exe")) {
    Write-Error "Build artifacts not found. Run 'cargo build --release' first."
    exit 1
}

Write-Host "=== Installing CheIME ===" -ForegroundColor Cyan

$cheimeDir = "$env:LOCALAPPDATA\CheIME"
$binDir = "$cheimeDir\bin"
$dataDir = "$cheimeDir\data\dicts"
$configDir = "$cheimeDir\config"

New-Item -ItemType Directory -Force -Path $binDir | Out-Null
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
New-Item -ItemType Directory -Force -Path $configDir | Out-Null

Copy-Item -Force "$releaseDir\cheime-engine.exe" $binDir
Copy-Item -Force "$releaseDir\cheime_tip.dll" "$binDir\cheime-tip.dll"
Copy-Item -Force "$releaseDir\cheime-installer.exe" $binDir
Copy-Item -Force "data\dicts\*" $dataDir

Write-Host "  Files copied to $cheimeDir" -ForegroundColor Green

Write-Host "  Registering TIP DLL..." -ForegroundColor Yellow
$dllPath = "$binDir\cheime-tip.dll"
regsvr32.exe /s $dllPath

$clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
$regPath = "Registry::HKEY_CLASSES_ROOT\CLSID\$clsid\InprocServer32"
if (Test-Path $regPath) {
    Write-Host "  TIP registered successfully" -ForegroundColor Green
} else {
    Write-Warning "  TIP may not have registered correctly. Try running as administrator."
}

Write-Host ""
Write-Host "=== Installation complete ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "To start the engine: $binDir\cheime-engine.exe --dict-dir $dataDir"
Write-Host "To uninstall: $binDir\cheime-installer.exe uninstall"
