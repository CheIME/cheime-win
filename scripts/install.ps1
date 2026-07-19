# CheIME Windows install script
# Requires admin for HKLM registration. Run from project root or scripts/ directory.

$ErrorActionPreference = "Stop"

$distDir = "target\release\dist"
if (-not (Test-Path $distDir)) {
    $distDir = "..\target\release\dist"
}
if (-not (Test-Path $distDir)) {
    Write-Error "Build artifacts not found. Run build.ps1 first."
    exit 1
}

Write-Host "=== Installing CheIME ===" -ForegroundColor Cyan

# Destination directories
$cheimeDir = "$env:LOCALAPPDATA\CheIME"
$binDir = "$cheimeDir\bin"
$dataDir = "$cheimeDir\data\dicts"
$configDir = "$cheimeDir\config"

New-Item -ItemType Directory -Force -Path $binDir | Out-Null
New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
New-Item -ItemType Directory -Force -Path $configDir | Out-Null

# Copy files
Copy-Item -Force "$distDir\cheime-engine.exe" $binDir
Copy-Item -Force "$distDir\cheime-tip.dll" $binDir
Copy-Item -Force "$distDir\cheime-installer.exe" $binDir
Copy-Item -Force "$distDir\data\dicts\*" $dataDir

Write-Host "  Files copied to $cheimeDir" -ForegroundColor Green

# Register the TIP DLL
Write-Host "  Registering TIP DLL..." -ForegroundColor Yellow
regsvr32.exe /s "$binDir\cheime-tip.dll"
if ($LASTEXITCODE -eq 0) {
    Write-Host "  TIP registered successfully" -ForegroundColor Green
} else {
    # regsvr32 might not set exit code correctly, check registry
    $clsid = "{B5F1C9A8-3E7D-4A15-AE2D-F89C1B6E3A07}"
    $regPath = "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32"
    if (Test-Path $regPath) {
        Write-Host "  TIP registration verified via registry" -ForegroundColor Green
    } else {
        Write-Warning "  TIP may not have registered correctly. Try running as administrator."
    }
}

Write-Host ""
Write-Host "=== Installation complete ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "To start the engine: $binDir\cheime-engine.exe --dict-dir $dataDir"
Write-Host "To uninstall: $binDir\cheime-installer.exe uninstall"
Write-Host ""
Write-Host "After installation, CheIME should appear in:"
Write-Host "  Settings > Time & Language > Language & Region > Chinese (Simplified)"
Write-Host "  > Language options > Add a keyboard"
